## Plan Complete: Scaffold Inference Subsystem

The `inference` Rust subsystem has been fully scaffolded from zero to `cargo check` clean with 35 passing tests. It is the exclusive owner of `llama-cpp-rs` in Sena, providing model loading, request queuing, and gRPC-based inference serving with a complete boot sequence and graceful shutdown.

**Phases Completed:** 8 of 8
1. ✅ Phase 1: Cargo.toml, build.rs, proto codegen, manifest
2. ✅ Phase 2: error.rs — InferenceError enum with gRPC Status mapping
3. ✅ Phase 3: config.rs — TOML config with validation
4. ✅ Phase 4: model_registry.rs — Model state tracking, VRAM allocation, LRU
5. ✅ Phase 5: model_loader.rs — llama-cpp-rs wrapper with spawn_blocking
6. ✅ Phase 6: request_queue.rs — Priority-aware async request queue
7. ✅ Phase 7: inference_engine.rs + grpc.rs — Core engine and InferenceService gRPC
8. ✅ Phase 8: main.rs — Boot sequence and graceful shutdown

**All Files Created/Modified:**
- inference/Cargo.toml
- inference/build.rs
- inference/manifest.toml
- inference/config/inference.toml
- inference/src/main.rs
- inference/src/error.rs
- inference/src/config.rs
- inference/src/model_registry.rs
- inference/src/model_loader.rs
- inference/src/request_queue.rs
- inference/src/inference_engine.rs
- inference/src/grpc.rs
- inference/src/generated/sena.daemonbus.v1.rs

**Key Functions/Classes Added:**
- `InferenceError` — 13-variant error enum with exhaustive `From<InferenceError> for tonic::Status`
- `Config` — TOML config with GrpcConfig, ModelConfig, RuntimeConfig, LoggingConfig + validation
- `ModelRegistry` — Arc<RwLock<>> model state tracker with LRU eviction
- `ModelHandle` / `ModelLoader::load/unload` — llama-cpp-rs wrapper in spawn_blocking
- `RequestQueue` — BinaryHeap + Notify priority queue with timeout expiry
- `InferenceEngine` — orchestrates registry, loader, queue, and worker loop
- `InferenceGrpcService` — implements InferenceService trait (complete, stream_complete, read_activations, steer)
- `async_main()` — 10-step boot sequence with daemon-bus integration
- `wait_for_daemon_bus_ready()` / `best_effort_signal()` / `initialize_tracing()` — boot helpers

**Test Coverage:**
- Total tests written: 35
- All tests passing: ✅

**Recommendations for Next Steps:**
- Replace FIXME(inference) placeholder in `process_request` with actual llama-cpp-2 completion API calls
- Replace FIXME(inference) word-splitting placeholder in `stream_complete` with real token-level streaming
- Implement `read_activations` and `steer` RPCs (currently return `Status::unimplemented`)
- Replace VRAM estimation heuristic with proper GGUF metadata parsing
- Add integration tests with a real model file
- Add telemetry spans (OpenTelemetry) per the PRD observability requirements
