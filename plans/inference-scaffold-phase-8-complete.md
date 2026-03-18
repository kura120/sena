## Phase 8 Complete: Boot Sequence and Graceful Shutdown

Implemented the full 10-step boot sequence in `main.rs` with config-driven tracing, daemon-bus integration, LlamaBackend initialization, model loading, gRPC server startup, and graceful shutdown that signals INFERENCE_UNAVAILABLE before exit.

**Files created/changed:**
- inference/src/main.rs
- inference/src/config.rs
- inference/config/inference.toml
- inference/src/inference_engine.rs (test fixtures)
- inference/src/grpc.rs (test fixtures)

**Functions created/changed:**
- `main()` — builds tokio runtime, delegates to async_main
- `async_main()` — full 10-step boot sequence returning exit code
- `wait_for_daemon_bus_ready()` — subscribes to event bus, waits for DAEMON_BUS_READY
- `best_effort_signal()` — non-fatal boot signal helper
- `initialize_tracing()` — json/pretty tracing subscriber from config
- `GrpcConfig::listen_address` — configurable bind address (new field)

**Tests created/changed:**
- `test_boot_signal_gate_prevents_early_start` — verifies oneshot gate pattern
- `test_shutdown_signal_channel_works` — verifies shutdown channel resolves
- Updated all test config fixtures in config.rs, inference_engine.rs, grpc.rs for new listen_address field

**Review Status:** APPROVED with revisions applied (removed hardcoded bind address, added justification comments on let _ assignments)

**Git Commit Message:**
feat(inference): implement boot sequence and graceful shutdown
