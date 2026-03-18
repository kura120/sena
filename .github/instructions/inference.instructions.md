---
applyTo: "inference/**"
---

# inference — Copilot Instructions

The inference subsystem owns `llama-cpp-rs` exclusively. It is the only process in Sena that loads or runs a completion model. Every other subsystem that needs completions is a gRPC client to `InferenceService`. It signals `INFERENCE_READY` on daemon-bus when the model is loaded and ready.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Language

**Rust only.** All async is tokio. The model is loaded via `llama-cpp-rs`. CPU-bound inference work runs on a dedicated thread via `tokio::task::spawn_blocking` — never block the tokio runtime with inference calls. Logging uses `tracing`.

---

## Ownership Boundaries

inference owns:
- The `llama-cpp-rs` model instance — exclusively. No other subsystem touches it.
- `InferenceService` gRPC — `Complete` and `StreamComplete` RPCs
- The model registry — tracking which GGUF is loaded, VRAM allocation, and load state
- Runtime model switching — unload, load new GGUF, re-signal `INFERENCE_READY`
- OOM detection and graceful degradation
- Activation hook surface — `ReadActivations` and `Steer` RPCs reserved in proto (Phase 2/3 implementation, Phase 1 proto stubs only)

inference does not own:
- Prompt assembly — that is prompt-composer
- Memory reads — that is memory-engine
- Model capability detection — model-probe calls `InferenceService` gRPC to run its probe battery
- Embedding — memory-engine loads a separate embedding GGUF (nomic-embed) in-process

---

## Model Loading Traps

### Only One Completion Model Loaded at a Time on Low Tier

On Low hardware tier (4–6GB VRAM), the inference subsystem loads exactly one completion model. Never attempt to load a second completion model before unloading the first. Always check `HardwareProfile.tier` from model-probe before deciding load strategy.

### VRAM Budget Is Always Checked Before Loading

Before loading any model, query available VRAM and compare against the model's estimated footprint. If the model would exceed the VRAM budget, return `Err` and log at `error` level. Never attempt a load that will OOM.

```rust
// good — check VRAM budget before loading
let available_vram = hardware_profile.available_vram_mb;
let model_footprint = estimate_vram_mb(&model_path)?;
if model_footprint > available_vram {
    return Err(InferenceError::InsufficientVram { required: model_footprint, available: available_vram });
}
load_model(&model_path).await?;
```

### Model Load Is Always Followed By INFERENCE_READY

After a successful model load, emit `INFERENCE_READY` to daemon-bus. No other subsystem proceeds with inference calls until this signal fires. Never emit `INFERENCE_READY` before the model has fully initialized.

### Model Switch Sequence Is Fixed

Runtime model switching follows this exact sequence. Never deviate from it.

1. Emit `INFERENCE_UNAVAILABLE` to daemon-bus
2. Drain in-flight requests — return `UNAVAILABLE` to any callers
3. Unload current model — free VRAM
4. Load new GGUF
5. Emit `INFERENCE_READY`
6. model-probe reruns its probe battery
7. `MODEL_PROFILE_READY` emitted by model-probe

Active callers during switch receive `UNAVAILABLE` and are expected to retry after `INFERENCE_READY`.

---

## Inference Execution Traps

### Inference Runs on a Blocking Thread — Never the Tokio Runtime

`llama-cpp-rs` inference is synchronous and CPU/GPU bound. Always run it via `tokio::task::spawn_blocking`. Never call inference functions directly from an async context.

```rust
// bad — blocks the tokio runtime
async fn complete(&self, request: CompleteRequest) -> Result<CompleteResponse, InferenceError> {
    let output = self.model.generate(&request.prompt)?; // blocks runtime
    Ok(build_response(output))
}

// good — offloaded to blocking thread
async fn complete(&self, request: CompleteRequest) -> Result<CompleteResponse, InferenceError> {
    let model = Arc::clone(&self.model);
    let output = tokio::task::spawn_blocking(move || {
        model.generate(&request.prompt)
    })
    .await
    .map_err(|join_err| InferenceError::SpawnBlocking(join_err))??;
    Ok(build_response(output))
}
```

### OOM Is Handled Gracefully — Never a Silent Crash

If the model produces an OOM error during inference, inference subsystem must:
1. Log at `error` level with VRAM state
2. Emit `INFERENCE_DEGRADED` to daemon-bus
3. Return `Err` to the caller
4. Attempt to recover — if recovery fails, emit `INFERENCE_UNAVAILABLE`

Never let an OOM propagate as an unhandled panic.

### Concurrent Requests Are Serialized Through a Queue

llama-cpp-rs is not thread-safe for concurrent completions on the same model instance. All `Complete` calls are serialized through an internal queue. The queue respects the global priority tiers — `Reactive` (user-facing) preempts `Standard` (agent calls) which preempts `Background` (CTP).

---

## Activation Hook Reservation (Phase 1 — Stubs Only)

The `InferenceService` proto must reserve `ReadActivations` and `Steer` RPCs from day one, even though they are not implemented in Phase 1. This ensures the API surface is not closed off when activation steering is implemented in Phase 2/3.

```protobuf
// reserve in proto — Phase 1 stub, Phase 2/3 implementation
rpc ReadActivations(ActivationRequest) returns (ActivationResponse);
rpc Steer(SteeringRequest) returns (SteeringAck);
```

Phase 1 implementations return `Status::unimplemented("reserved for Phase 2")`. Never remove these RPCs. Never change their signatures without a proto breaking-change review.

The llama.cpp C API exposes activation access at the ggml graph level. The inference subsystem must not close off the internal hook points that would be needed for these RPCs. When designing the model execution path, leave the ggml tensor access points reachable — do not encapsulate them in a way that makes them inaccessible from `grpc.rs`.

---

## Model Registry Traps

### The Registry Tracks Load State — Not Just Model ID

The model registry is not a simple ID → path map. It tracks full load state per model: `Unloaded`, `Loading`, `Ready`, `Failed`, `Switching`. Never read from the model while its registry state is anything other than `Ready`.

### Multi-Model Loading Is Mid/High Tier Only

On Low tier, the registry has exactly one slot. On Mid/High tiers, multiple models may be loaded with LRU eviction when VRAM budget is exceeded. Always check `HardwareProfile.tier` before allowing multi-model load paths.

### Agents Declare Their Required Model — Inference Honors It

Agent manifests declare `model_id`. The inference subsystem resolves that ID against the registry. If the required model is not loaded and VRAM permits, load it. If VRAM does not permit, evict the LRU model first (Mid/High tier only). On Low tier, return `ModelNotAvailable` if the required model differs from the currently loaded one.

---

## Logging

Use `tracing` exclusively. Required fields:

- `event_type` — `model_loading`, `model_ready`, `model_unloading`, `inference_started`, `inference_completed`, `inference_failed`, `oom_detected`, `model_switching`
- `model_id` — always include on model lifecycle events
- `vram_used_mb` — include on load, unload, and OOM events
- `duration_ms` — include on inference completed and failed events

```rust
tracing::info!(
    event_type = "inference_completed",
    model_id = %self.active_model_id,
    duration_ms = elapsed.as_millis(),
    tokens_generated = output.token_count,
    priority = %request.priority,
);
```

OOM events: `error` level.
Model switching: `info` level at each step of the sequence.
Inference failures: `error` level with full error context.
Inference completions: `debug` level (high frequency — not `info`).