---
applyTo: "lora-manager/**"
---

# lora-manager — Copilot Instructions

lora-manager is Sena's idle-time LoRA adapter training subsystem. It trains lightweight weight-delta adapters on top of the frozen base model using accumulated local interaction data. It runs only during confirmed idle periods and never during active use. It is an extractable open-source subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

lora-manager owns:
- Subscribing to `LORA_TRAINING_RECOMMENDED` events from daemon-bus
- Evaluating whether all training trigger conditions are met before scheduling
- Training LoRA adapters on accumulated interaction data during idle windows
- Quality gating — running model-probe's reasoning quality probe before and after training
- Versioning and storing adapters under `~/.sena/lora/<model_id>/`
- Loading the active adapter at boot (step 5.6) after `MODEL_PROFILE_READY`
- Signalling `LORA_READY` or `LORA_SKIPPED` to daemon-bus after boot load attempt
- Archiving previous adapters on successful deployment
- Discarding rejected adapters without affecting the active adapter

lora-manager does not own:
- Detecting reasoning gaps — that is model-probe
- Probing model capabilities — that is model-probe
- Memory reads beyond the interaction dataset it needs for training
- Any model inference during active user sessions
- SoulBox state — lora-manager is stateless between training runs

---

## Training Trigger Traps

### All Conditions Must Pass — Never Train Partially
Training only begins when every trigger condition is satisfied simultaneously. Never start a training run if any condition fails.

```python
# good — all conditions checked before scheduling
async def should_train(self, model_id: str) -> bool:
    return (
        await self._is_deep_idle()
        and await self._resources_available()
        and await self._sufficient_new_interactions(model_id)
        and await self._training_recommended_or_overdue(model_id)
        and await self._model_is_lora_compatible(model_id)
    )
```

### Trigger Conditions Come From Config — Never Hardcoded
Idle duration, resource thresholds, minimum interaction count, and overdue interval are all defined in `lora-manager/config.toml`. Never hardcode them.

```python
# bad
if idle_minutes >= 10 and cpu_pct < 30:
    ...

# good
if idle_minutes >= config.training.min_idle_minutes and cpu_pct < config.training.max_cpu_pct:
    ...
```

### Never Train During Active Interaction
lora-manager subscribes to activity state events from daemon-bus. If activity state changes to `active` during a training run, the run must be cancelled cleanly — not paused, cancelled. Resume from scratch at the next idle window.

```python
# good — cancel on activity resume
async def run_training(self, model_id: str) -> None:
    async with self._activity_guard() as guard:
        async for step in self._training_steps(model_id):
            if guard.interrupted:
                logger.info("training_cancelled_activity_resumed", model_id=model_id)
                return
            await step
```

---

## Adapter Lifecycle Traps

### Quality Gate Is Mandatory — Never Skip It
After training completes, model-probe's reasoning quality probe must be run with the new adapter loaded. If the score does not improve over the pre-training baseline, discard the adapter. Never deploy an adapter that has not passed the quality gate.

```python
# good — quality gate before deployment
baseline_score = profile.reasoning_quality_score
new_score = await model_probe.run_reasoning_quality_probe(model_id, adapter=new_adapter)

if new_score > baseline_score:
    await self._deploy_adapter(model_id, new_adapter)
    await daemon_bus.publish(events.LORA_ADAPTER_UPDATED, {"model_id": model_id, "score": new_score})
else:
    await self._discard_adapter(new_adapter)
    await daemon_bus.publish(events.LORA_ADAPTER_REJECTED, {"model_id": model_id, "score": new_score})
```

### Previous Adapter Is Archived — Never Deleted on Deploy
When a new adapter is deployed, archive the previous one. Never delete previous versions. The user may revert.

```python
# good — archive before replacing
archive_path = adapter_dir / f"{model_id}_v{current_version}.adapter"
current_adapter_path.rename(archive_path)
new_adapter_path.rename(current_adapter_path)
```

### Adapters Are Architecture-Specific — Never Cross-Load
An adapter trained on llama3.1 cannot be loaded on mistral. Always verify that the adapter's recorded model architecture matches the currently active model before loading.

```python
# good — architecture check before load
adapter_meta = load_adapter_metadata(adapter_path)
if adapter_meta.architecture != active_model_architecture:
    logger.info("adapter_skipped_architecture_mismatch",
        adapter_arch=adapter_meta.architecture,
        active_arch=active_model_architecture)
    await daemon_bus.publish(events.LORA_SKIPPED, {"reason": "architecture_mismatch"})
    return
```

### No Adapter Is Not an Error
If no adapter exists for the current model, Sena runs on the base model. This is normal on first use or after a model swap. Never raise or signal an error when no adapter is found — signal `LORA_SKIPPED` and continue.

---

## Model Swap Traps

### On Model Swap — Check Before Training
When the active model changes, lora-manager must check for an existing adapter for the new model's architecture before queuing a new training run. Never queue training if a valid adapter already exists and passes the quality gate.

### On Model Swap to Incompatible Architecture — Disable Cleanly
If the new model is not LoRA-compatible (as reported by model-probe), lora-manager disables itself for that model, signals `LORA_SKIPPED`, and takes no further action until the model changes again.

---

## Storage Traps

### Adapter Storage Is Encrypted
Adapters are stored encrypted alongside SoulBox. Use the same key derivation mechanism. Never store adapter files in plaintext.

### Adapter Metadata Is Stored Separately From Weights
Each adapter directory contains a `metadata.toml` alongside the weight file. The metadata records the model architecture, training date, interaction record count, pre-training score, and post-training score.

```toml
# ~/.sena/lora/llama3.1/metadata.toml
architecture = "llama"
model_id = "llama3.1:latest"
trained_at = "2025-03-08T14:32:00Z"
interaction_count = 127
pre_training_score = 0.61
post_training_score = 0.74
version = 3
```

### Delete SoulBox Deletes All Adapters
lora-manager listens for `SOULBOX_DELETED` events from daemon-bus and purges the entire `~/.sena/lora/` directory on receipt.

---

## Concurrency Rules

lora-manager is single-threaded by design. Only one training run may be active at any time. Training is CPU-bound — use `ProcessPoolExecutor` via `run_in_executor` for the training loop itself to avoid blocking the asyncio event loop while still allowing the activity state monitor to interrupt.

```python
# good — training in executor, event loop free to receive interrupts
loop = asyncio.get_event_loop()
await loop.run_in_executor(self._training_executor, self._run_training_sync, dataset)
```

---

## Logging

Use `structlog` exclusively. Required fields on every lora-manager log event:

- `event_type` — training_scheduled, training_started, training_cancelled, training_completed, adapter_deployed, adapter_rejected, adapter_loaded, adapter_skipped
- `model_id` — always include
- `reason` — always include on skip, cancel, or reject events

```python
logger.info(
    "adapter_deployed",
    model_id=model_id,
    pre_score=baseline_score,
    post_score=new_score,
    version=new_version
)
```

Training start and completion must be logged at `info` level.
Quality gate rejections must be logged at `info` level with both scores.
Cancellations must be logged at `info` level with reason.
Storage or encryption errors must be logged at `error` level.
