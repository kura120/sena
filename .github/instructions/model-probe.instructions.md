---
applyTo: "model-probe/**"
---

# model-probe — Copilot Instructions

model-probe is Sena's runtime model capability detection subsystem. It runs once at boot — after `INFERENCE_READY` fires, before CTP starts. It probes the active model with a battery of lightweight test prompts via `InferenceService` gRPC, builds a `ModelCapabilityProfile` and a `HardwareProfile`, publishes both to daemon-bus, signals `MODEL_PROFILE_READY`, and exits cleanly.

model-probe is stateless — it holds no data between runs. It is an extractable open-source subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Language

**Rust only.** All probe calls go through `InferenceService` gRPC — never directly to llama-cpp-rs or any model runtime. No Python. No Ollama. Logging uses `tracing`.

---

## Ownership Boundaries

model-probe owns:
- Running the probe battery via `InferenceService.Complete` gRPC
- Hardware detection — VRAM, RAM, CPU core count via platform APIs
- Scoring each probe result against configured thresholds
- Building and publishing `ModelCapabilityProfile` and `HardwareProfile`
- Signalling `MODEL_PROFILE_READY` to daemon-bus
- Re-running the full battery when `INFERENCE_READY` fires again (model switch)
- Publishing `LORA_TRAINING_RECOMMENDED` to daemon-bus when a reasoning gap is detected

model-probe does not own:
- Deciding what to do with capability limitations — that is prompt-composer and CTP
- Communicating limitations to the user — that is the reactive loop
- Model loading, unloading, or switching — that is inference
- Training or managing LoRA adapters — that is lora-manager
- Any memory reads beyond publishing the capability profile

---

## InferenceService Interaction Traps

### All Probe Calls Go Through InferenceService.Complete

model-probe is a gRPC client to inference. Never call llama-cpp-rs directly, never spawn a model process, never use any HTTP model API.

```rust
// bad — direct model access
let output = llama_model.generate(&prompt)?;

// good — gRPC call to InferenceService
let response = inference_client
    .complete(CompleteRequest {
        prompt: probe_prompt.into(),
        model_id: String::new(), // empty = active model
        max_tokens: 100,
        temperature: 0.0,
        priority: Priority::Standard as i32,
        request_id: Uuid::new_v4().to_string(),
    })
    .await?
    .into_inner();
```

### LoRA Compatibility Is Read From Model Metadata — Not Inferred

The LoRA compatibility check reads architecture metadata from the `ModelCapabilityProfile`'s model_id field and matches it against the configured list of compatible architectures. It does not run a completion call. It does not call any external API. Architecture families and their LoRA compatibility are loaded from `config/model-probe.toml` — never hardcoded.

### model-probe Waits For INFERENCE_READY Before Starting

model-probe must not begin the probe battery until it has received `INFERENCE_READY` from daemon-bus. The boot sequence enforces this, but the implementation must also guard against early starts.

---

## Probe Design Traps

### Probes Are Lightweight — Under 2 Seconds Each

Every probe must complete in under 2 seconds on modest hardware. model-probe runs on every boot and on every model switch. An expensive probe makes Sena feel slow to start.

```rust
// bad — open-ended prompt with no token cap
CompleteRequest { prompt: "Explain the nature of consciousness in depth".into(), max_tokens: 2048, .. }

// good — minimal prompt, capped tokens, temperature 0
CompleteRequest {
    prompt: REASONING_PROBE_PROMPT.into(),
    max_tokens: 100,
    temperature: 0.0,
    ..
}
```

### Probes Use Deterministic Settings

Always run probes at `temperature: 0.0` with a fixed `max_tokens` cap from config. Probes must be reproducible. A probe that passes at temperature 0.7 but fails at 0.0 is not a passing probe.

### Probes Have Known Expected Outputs

Every probe has a known correct answer or format that can be scored programmatically. Never design a probe whose result requires subjective judgment.

```rust
// bad — subjective scoring
let score = evaluate_quality(&response.text);

// good — deterministic scoring against known answer
let answer = extract_numeric_answer(&response.text);
let passed = answer == config.probes.reasoning.expected_answer;
```

### Probes Are Independent — Run in Parallel

Each probe must be runnable independently. Never design probe B to depend on probe A's result. Run all probes concurrently with `tokio::join!` or `futures::future::join_all`.

---

## Probe Battery

All probes are required on every run. Never skip probes selectively.

| Probe | What It Tests | Effect on Sena |
|---|---|---|
| Structured output | Can the model reliably produce TOON-formatted output? | PC uses TOON vs JSON fallback |
| Multi-step reasoning | 3-step logical inference with known answer | Reasoning agent enabled or disabled |
| Tool / function calling | Simple tool call with known expected output | Agent tool use enabled or disabled |
| Context retention | Retention tested at 25%, 50%, 75% of advertised limit | Sets conservative effective context budget |
| Response coherence | Known question with known answer, scored for similarity | Sets quality floor for CTP relevance threshold |
| Instruction following | Structured task with precise expected format | Determines PC formatting strictness |
| Reasoning quality baseline | Open-ended contextual inference scored against expected reasoning chain | Sets LoRA training threshold and reasoning gap baseline |
| LoRA compatibility | Architecture metadata check against known compatible families | Gates lora-manager |
| Memory injection fidelity | Injects a known memory context, tests whether model reasons from it | Sets memory injection depth for PC |
| Reasoning gap detection | Compares current quality score against score at last LoRA run | Publishes `LORA_TRAINING_RECOMMENDED` if gap exceeds threshold |

### Reasoning Quality Baseline Probe

Produces a `f32` score between 0.0 and 1.0 that is stable and comparable across runs. This score is the baseline against which reasoning gap detection computes drift.

```rust
let score = run_reasoning_quality_probe(&mut inference_client, &config).await?;
profile.reasoning_quality_score = score;
profile.reasoning_quality_probed_at = Utc::now();
```

### LoRA Compatibility Probe

Architecture check only — no completion call. Reads the model identifier, extracts the architecture family, cross-references against the configured list.

```rust
// compatible families come from config — never hardcoded
let family = extract_architecture_family(&model_id);
profile.lora_compatible = config.lora_compatible_architectures.contains(&family);
```

### Reasoning Gap Detection

Compares the current `reasoning_quality_score` against the score recorded at the last LoRA training run. If the gap exceeds the configured threshold and the model is LoRA-compatible, publish `LORA_TRAINING_RECOMMENDED`.

```rust
if let Some(last_score) = last_lora_training_score {
    let gap = last_score - profile.reasoning_quality_score;
    if gap > config.probes.reasoning_gap.trigger_threshold && profile.lora_compatible {
        daemon_bus_client
            .publish(PublishRequest {
                topic: Topic::LoraTrainingRecommended as i32,
                payload: serialize_gap_event(model_id, profile.reasoning_quality_score, last_score, gap)?,
            })
            .await?;
    }
}
```

model-probe never trains adapters. It detects and signals. lora-manager owns all training decisions.

---

## Scoring Traps

### Thresholds Are Config — Never Hardcoded

Pass/fail thresholds for each probe are defined in `config/model-probe.toml`. Never hardcode a threshold value.

```rust
// bad
if coherence_score > 0.75 { .. }

// good
if coherence_score > config.probes.coherence.pass_threshold { .. }
```

### Capabilities Are Graduated — Not Binary

Never score a capability as simply pass/fail. Use graduated levels so prompt-composer and CTP can make nuanced decisions.

```rust
pub enum CapabilityLevel {
    None = 0,     // probe failed completely
    Degraded = 1, // probe passed partially — use with fallbacks
    Full = 2,     // probe passed — full capability available
}
```

### Context Window Is Conservative

Always set the effective context budget to the lowest retention level that passed. Never use the advertised context window size.

```rust
// bad — uses advertised limit
profile.effective_context_window = advertised_context_length;

// good — uses proven retention level
profile.effective_context_window = highest_passing_retention_window;
```

---

## Re-probe Traps

### Re-probe Is Always Full — Never Partial

When the model changes (`INFERENCE_READY` fires again), run the full probe battery. Never selectively re-run only some probes.

### Re-probe Blocks CTP Start

CTP must not start until `MODEL_PROFILE_READY` is emitted. The boot sequence enforces this. Never bypass or skip the signal.

### Stale Profiles Are Never Used

If the probe battery fails (timeout, `InferenceService` error), do not use a cached profile from a previous run. Fail clearly and notify daemon-bus.

```rust
// bad — falls back to stale profile
Err(_) => load_cached_profile(),

// good — fails clearly, enters minimal capability mode
Err(error) => {
    tracing::error!(event_type = "probe_battery_failed", %error);
    daemon_bus_client.publish(/* MODEL_PROBE_FAILED */).await?;
    ModelCapabilityProfile::minimal()
}
```

---

## ModelCapabilityProfile and HardwareProfile

Never add fields to either profile without updating `shared/proto/` first.

```rust
pub struct ModelCapabilityProfile {
    pub model_id: String,
    pub probed_at: DateTime<Utc>,
    pub structured_output: CapabilityLevel,
    pub multi_step_reasoning: CapabilityLevel,
    pub tool_calling: CapabilityLevel,
    pub effective_context_window: u32,
    pub coherence_baseline: f32,
    pub instruction_following: CapabilityLevel,
    pub reasoning_quality_score: f32,
    pub reasoning_quality_probed_at: DateTime<Utc>,
    pub lora_compatible: bool,
    pub memory_injection_depth: CapabilityLevel,
}

pub struct HardwareProfile {
    pub tier: HardwareTier, // Low | Mid | High
    pub vram_mb: u32,
    pub ram_mb: u32,
    pub cpu_cores: u32,
    pub gpu_name: String,
}

pub enum HardwareTier {
    Low,  // 4–6GB VRAM
    Mid,  // 8–12GB VRAM
    High, // 16GB+ VRAM
}
```

`HardwareTier` is derived from detected VRAM at runtime. Thresholds come from config — never hardcoded.

---

## Logging

Use `tracing` exclusively. Required fields on every probe log event:

- `probe_name` — which probe ran
- `model_id` — which model was probed
- `result` — `full`, `degraded`, or `none`
- `score` — raw numeric score before threshold comparison
- `duration_ms` — how long the probe took

```rust
tracing::info!(
    event_type = "probe_completed",
    probe_name = "multi_step_reasoning",
    model_id = %model_id,
    result = %capability_level,
    score = raw_score,
    duration_ms = elapsed.as_millis(),
);
```

Full probe battery completion: `info` level with a summary of all capability levels and hardware tier.
`LORA_TRAINING_RECOMMENDED` events: `info` level with gap score included.
Probe failures: `error` level with full error context.