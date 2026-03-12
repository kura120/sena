# PLAN — model-probe scaffolding

## What

Scaffold the complete model-probe subsystem: a stateless Rust binary that runs once at boot, probes the active model's capabilities via llama-cpp-rs, detects hardware tier, publishes profiles to daemon-bus via gRPC, and exits.

## Why

model-probe is boot step 5.5 — it must run after the model is loaded and before CTP starts. Every downstream subsystem (PC, CTP, agents) depends on the `ModelCapabilityProfile` and `HardwareProfile` to gate behaviors. Without this subsystem, Sena cannot determine what the active model can do.

## Subsystems affected

- **model-probe** (primary) — all files created from scratch
- **daemon-bus** (consumed, not modified) — model-probe imports proto-generated client types from `daemon-bus/proto/sena.daemonbus.v1.proto` to signal `MODEL_PROFILE_READY`, publish `LORA_TRAINING_RECOMMENDED`, and report failures via `TOPIC_MODEL_PROBE_FAILED`

## Assumptions

- **llama-cpp-rs** is the inference backend. We depend on the `llama-cpp-2` crate (Rust bindings for llama.cpp). Probe stubs are written against its API shape but actual inference calls are stubbed until the model loading infrastructure is integrated.
- **Proto types are consumed as a path dependency** from `../daemon-bus`. model-probe's `build.rs` compiles the same proto file and generates client-only code.
- **Hardware detection** uses `nvml-wrapper` for NVIDIA GPU info (VRAM, CUDA compute) and `sysinfo` for RAM. These are the standard Rust crates for this purpose on Windows.
- **Config lives at `model-probe/config/model-probe.toml`** and is loaded at startup. All thresholds, timeouts, model paths, and scoring weights come from this file.
- **model-probe is a standalone binary** (`[[bin]]`), not a library. It connects to daemon-bus as a gRPC client, runs probes, publishes results, and exits.
- **CapabilityLevel uses three tiers**: `Full`, `Partial`, `None` — matching the PRD's graduated scoring. The instruction file's `DEGRADED` is mapped to `Partial` to align with the task spec's enum definition.
- **No memory-engine dependency at scaffold time**. The reasoning gap detection compares against a `last_lora_training_score` that would come from memory-engine. For the scaffold, this is stubbed as `None` (first run scenario — no prior training score exists).
- **Probes run concurrently where independent** via `tokio::join!`. The reasoning gap detection depends on the reasoning quality probe result, so those run sequentially.

## Out of scope

- Actual llama-cpp-rs model loading and inference execution (stubs only)
- memory-engine gRPC client for retrieving last LoRA training score (stubbed)
- Integration testing against a running daemon-bus instance
- OpenTelemetry trace propagation setup (spans are emitted but exporter is not configured)
- Re-probe on model change (requires daemon-bus subscription, not part of boot-once scaffold)

## Deliverables

| File | Purpose |
|---|---|
| `Cargo.toml` | Crate manifest with all dependencies |
| `build.rs` | Proto codegen from daemon-bus proto |
| `config/model-probe.toml` | Default configuration with all thresholds |
| `src/main.rs` | Entry point: load config, detect hardware, connect gRPC, run probes, publish, exit |
| `src/config.rs` | Strongly-typed TOML config deserialization |
| `src/error.rs` | SenaError with ErrorCode, cross-process stripping |
| `src/hardware.rs` | VRAM/RAM detection, HardwareTier derivation |
| `src/probes.rs` | Concurrent probe runner, profile assembly |
| `src/probe/context_window.rs` | Context window / pre-rot threshold probe |
| `src/probe/structured_output.rs` | Structured output (KnowledgeGraph) probe |
| `src/probe/instruction_following.rs` | Multi-step instruction compliance probe |
| `src/probe/reasoning.rs` | Reasoning quality baseline + gap detection |
| `src/probe/lora_compat.rs` | LoRA architecture compatibility check |
| `src/probe/memory_fidelity.rs` | Memory injection fidelity probe |
| `src/probe/graph_extraction.rs` | Graph extraction capability probe |