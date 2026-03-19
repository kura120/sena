## Phase 4 Complete: Build Verification

The entire workspace builds cleanly with `cargo check --workspace` and `cargo build --workspace`. All five subsystem binaries are produced in `target/debug/` with current timestamps. No compile errors were found across any crate.

**Files created/changed:**
- `daemon-bus/src/grpc/event_bus_service.rs` — NEW: EventBusService gRPC handler bridging internal bus to gRPC
- `daemon-bus/src/grpc/mod.rs` — Added EventBusService registration and `event_bus` parameter
- `daemon-bus/src/main.rs` — Fixed config default path; pass `event_bus.clone()` to `start_grpc_server`
- `daemon-bus/config/daemon-bus.toml` — Added `model_probe`/`ctp` supervisor entries; raised timeouts; removed lora_manager from boot
- `daemon-bus/src/supervisor/mod.rs` — Fixed infinite restart loop (prior_restart_count read from state)
- `ctp/src/main.rs` — Removed LORA_READY from boot gate; updated test

**Functions created/changed:**
- `EventBusServiceHandler::new` — NEW
- `EventBusServiceHandler::publish` — NEW (gRPC → internal bus)
- `EventBusServiceHandler::subscribe` — NEW (internal bus → gRPC server-stream via ReceiverStream)
- `start_grpc_server` — Signature extended with `event_bus: EventBus`; registers `EventBusServiceServer`
- `attempt_restart_with_policy` — Reads `prior_restart_count` from stored process state before looping
- `wait_for_boot_prerequisites` (ctp) — Waits for 2 signals only (removed LORA dependency)

**Tests created/changed:**
- `test_boot_gate_requires_two_signals` (ctp) — Updated from 3-signal to 2-signal gate

**Review Status:** Verified — cargo check passes clean across entire workspace

**Git Commit Message:**
```
fix(daemon-bus): expose EventBusService and fix boot deadlocks

- Add EventBusService gRPC handler bridging internal broadcast bus
- Register EventBusServiceServer alongside BootServiceServer in grpc/mod.rs
- Add model_probe and ctp entries to supervisor.subsystems in daemon-bus.toml
- Remove lora_manager from boot.subsystems (never spawned; was blocking CTP)
- Fix config default path so daemon-bus runs from workspace root
- Fix attempt_restart_with_policy to respect prior restart count (no infinite loop)
- Remove LORA_READY from CTP boot gate (reduced to 2-signal: memory_engine + model_profile)
```
