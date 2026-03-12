---
applyTo: "model-probe/**"
---

# model-probe — Copilot Instructions

model-probe is Sena's runtime model capability detection subsystem. It runs at boot step 5.5 — after Ollama signals ready, before CTP starts. It probes the configured model with a battery of lightweight test prompts and builds a `ModelCapabilityProfile` that is stored in memory-engine and read by PC and CTP to gate which behaviors and agents are active. model-probe is an extractable open-source subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

model-probe owns:
- Running the probe battery against the active Ollama model
- Scoring each probe result against defined thresholds
- Building and publishing the `ModelCapabilityProfile`
- Re-running the battery when the model configuration changes
- Communicating ModelCapabilityProfile to memory-engine via gRPC
- Publishing `LORA_TRAINING_RECOMMENDED` to daemon-bus when a reasoning gap is detected
- Detecting LoRA architecture compatibility for the active model

model-probe does not own:
- Deciding what to do with capability limitations — that is PC and CTP
- Communicating limitations to the user — that is Sena's reactive loop
- Any memory reads beyond storing and updating the capability profile
- Model configuration itself — that is user-defined via Ollama
- Training or managing LoRA adapters — that is lora-manager

---

## Probe Design Traps

### Probes Are Lightweight — Never Expensive
Every probe must complete in under 2 seconds on modest hardware. model-probe runs on every boot and on every model change. An expensive probe makes Sena feel slow to start.

```python
# bad — open-ended prompt that may run long
result = await ollama.generate("Explain the nature of consciousness in depth")

# good — minimal prompt with known expected format
result = await ollama.generate(
    prompt=REASONING_PROBE_PROMPT,
    options={"max_tokens": 100, "temperature": 0}
)
```

### Probes Use Deterministic Settings
Always run probes at `temperature=0` with a fixed `max_tokens` cap. Probes must be reproducible. A probe that passes at temperature 0.7 but fails at 0 is not a passing probe.

### Probes Have Known Expected Outputs
Every probe has a known correct answer or format that can be scored programmatically. Never design a probe whose result requires subjective judgment.

```python
# bad — subjective scoring
result = await run_probe("Write a short poem")
score = evaluate_quality(result)

# good — deterministic scoring
REASONING_PROBE_ANSWER = 42
result = await run_probe(REASONING_PROBE_PROMPT)
passed = extract_answer(result) == REASONING_PROBE_ANSWER
```

### Probes Are Independent — Never Sequential Dependencies
Each probe must be runnable independently. Never design probe B to depend on probe A's result. Probes run in parallel where possible.

---

## Probe Battery

The full probe battery. All probes are required on every run — never skip probes selectively.

| Probe | What It Tests | Effect on Sena |
|---|---|---|
| Structured output | Can the model reliably produce TOON-formatted output? | PC uses TOON vs JSON fallback |
| Multi-step reasoning | 3-step logical inference with known answer | Reasoning agent enabled or disabled |
| Tool / function calling | Simple tool call with known expected output | Agent tool use enabled or disabled |
| Context retention | Retention tested at 25%, 50%, 75% of advertised limit | Sets conservative effective context budget |
| Response coherence | Known question with known answer, scored for similarity | Sets quality floor for CTP relevance threshold |
| Instruction following | Structured task with precise expected format | Determines PC formatting strictness |
| Reasoning quality baseline | Open-ended contextual inference scored against expected reasoning chain | Sets LoRA training threshold and reasoning gap detection baseline |
| LoRA compatibility | Checks model architecture against known LoRA-compatible families | Gates lora-manager — incompatible architectures skip adapter training |
| Memory injection fidelity | Injects a known memory context and tests whether the model reasons from it correctly | Sets memory injection depth for PC |
| Reasoning gap detection | Compares current reasoning quality score against score at last LoRA training run | Publishes `LORA_TRAINING_RECOMMENDED` to daemon-bus if gap exceeds threshold |

### Reasoning Quality Baseline Probe
This probe establishes the `reasoning_quality_score` stored in the capability profile. It must produce a numeric score between 0.0 and 1.0 that is stable and comparable across runs. The score is the baseline against which reasoning gap detection computes drift.

```python
# good — stable numeric score
score = await run_reasoning_quality_probe(model_id)
profile.reasoning_quality_score = score
profile.reasoning_quality_probed_at = utcnow()
```

### LoRA Compatibility Probe
This probe checks the model identifier against the known list of LoRA-compatible architectures. It does not run a model inference call — it is a structural check against the model's architecture metadata returned by Ollama.

```python
# good — architecture check, not inference
model_info = await ollama.show(model_id)
architecture = extract_architecture(model_info)
profile.lora_compatible = architecture in config.lora_compatible_architectures
# compatible list: llama, mistral, qwen, gemma, phi — loaded from config.toml, never hardcoded
```

### Reasoning Gap Detection
Reasoning gap detection compares the current `reasoning_quality_score` against the score recorded at the last LoRA training run. If the gap exceeds the configured threshold and the model is LoRA-compatible, publish `LORA_TRAINING_RECOMMENDED`.

```python
# good — gap detection after scoring
current_score = profile.reasoning_quality_score
last_trained_score = await memory_engine.get_last_lora_training_score(model_id)
gap = last_trained_score - current_score if last_trained_score else 0.0

if gap > config.probes.reasoning_gap.trigger_threshold and profile.lora_compatible:
    await daemon_bus.publish(events.LORA_TRAINING_RECOMMENDED, {
        "model_id": model_id,
        "current_score": current_score,
        "last_trained_score": last_trained_score,
        "gap": gap
    })
```

model-probe never trains adapters. It detects and signals. lora-manager owns all training decisions.

---

## Scoring Traps

### Thresholds Are Config — Never Hardcoded
Pass/fail thresholds for each probe are defined in `model-probe/config.toml`. Never hardcode a threshold value in probe logic.

```python
# bad
if coherence_score > 0.75:
    profile.structured_output = CapabilityLevel.FULL

# good
threshold = config.probes.structured_output.pass_threshold
if coherence_score > threshold:
    profile.structured_output = CapabilityLevel.FULL
```

### Capabilities Are Graduated — Not Binary
Never score a capability as simply pass/fail. Use graduated levels so PC and CTP can make nuanced decisions.

```python
class CapabilityLevel(Enum):
    NONE = 0        # probe failed completely
    DEGRADED = 1    # probe passed partially — use with fallbacks
    FULL = 2        # probe passed — full capability available
```

### Context Window Is Conservative
When probing practical context retention, always set the effective context budget to the lowest retention level that passed. Never use the advertised context window size.

```python
# bad — uses advertised limit
profile.effective_context_window = model_info.context_length

# good — uses proven retention level
profile.effective_context_window = highest_passing_retention_test_size
```

---

## Re-probe Traps

### Re-probe Is Always Full — Never Partial
When the model changes, run the full probe battery. Never selectively re-run only some probes — capability interactions between probes matter.

### Re-probe Blocks CTP Start
CTP must not start its loop until the ModelCapabilityProfile is published to memory-engine. The boot sequence enforces this via the `MODEL_PROFILE_READY` readiness signal. Never bypass this gate.

### Stale Profiles Are Never Used
If model-probe fails to complete (timeout, Ollama error), do not use a cached profile from a previous run. Fail loudly, log the error, and notify daemon-bus that Sena is starting in minimal capability mode.

```python
# bad — uses stale profile on failure
except ProbeError:
    profile = load_cached_profile()

# good — fails clearly
except ProbeError as error:
    logger.error("probe_battery_failed", error=str(error))
    await daemon_bus.publish(events.MODEL_PROBE_FAILED, {"reason": str(error)})
    profile = ModelCapabilityProfile.minimal()
```

---

## ModelCapabilityProfile Schema

The profile written to memory-engine must follow this structure exactly. Never add fields without updating `daemon-bus/proto/model_probe.proto` first.

```python
@dataclass
class ModelCapabilityProfile:
    model_id: str                          # Ollama model identifier
    probed_at: datetime                    # UTC timestamp of probe run
    structured_output: CapabilityLevel     # TOON encoding reliability
    multi_step_reasoning: CapabilityLevel  # Reasoning agent gate
    tool_calling: CapabilityLevel          # Agent tool use gate
    effective_context_window: int          # Conservative proven token limit
    coherence_baseline: float              # 0.0–1.0 response quality floor
    instruction_following: CapabilityLevel # PC formatting strictness
    reasoning_quality_score: float         # 0.0–1.0 baseline for gap detection
    reasoning_quality_probed_at: datetime  # UTC timestamp of last quality probe
    lora_compatible: bool                  # Whether architecture supports LoRA
    memory_injection_depth: CapabilityLevel # How much tiered context PC injects
```

---

## Logging

Use `structlog` exclusively. Required fields on every probe log event:

- `probe_name` — which probe ran
- `model_id` — which model was probed
- `result` — passed, degraded, or failed
- `score` — raw numeric score before threshold comparison
- `duration_ms` — how long the probe took

```python
logger.info(
    "probe_completed",
    probe_name="multi_step_reasoning",
    model_id=model_id,
    result=capability_level.name,
    score=raw_score,
    duration_ms=elapsed
)
```

Full probe battery completion must be logged at `info` level with a summary of all capability levels.
`LORA_TRAINING_RECOMMENDED` events must be logged at `info` level with gap score included.
Probe failures must be logged at `error` level with full exception context.
