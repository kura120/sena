## Plan: Inference Subsystem Scaffold

Scaffold the `inference` Rust subsystem — the exclusive owner of `llama-cpp-rs` in Sena. It serves `InferenceService` gRPC to all subsystems that need completions, manages VRAM budget and model lifecycle, serializes requests through a priority queue, and handles OOM and model switching gracefully. The proto contract is already committed.

**Phases 8**
1. **Phase 1: Foundation — Cargo.toml, manifest, build.rs, proto regeneration**
    - **Objective:** Get the crate defined, proto bindings regenerated, and `cargo check` passing with an empty `src/main.rs`.
    - **Files/Functions to Modify/Create:** `inference/Cargo.toml`, `inference/manifest.toml`, `inference/build.rs`, `inference/src/main.rs`, `inference/src/generated/`
    - **Tests to Write:** None — compilation is the acceptance criterion
    - **Steps:**
        1. Create `inference/Cargo.toml` with all required deps (`llama-cpp-2 = "0.1"`, tonic, prost, tokio, thiserror, serde, toml, uuid, tracing, tracing-subscriber, chrono, tokio-stream, ordered-float)
        2. Create `inference/manifest.toml`
        3. Create `inference/build.rs` — identical pattern to memory-engine/build.rs
        4. Create `inference/src/main.rs` with `fn main() {}`
        5. Create `inference/src/generated/sena.daemonbus.v1.rs` placeholder
        6. Run `cargo check -p inference` — fix until clean

2. **Phase 2: error.rs + config.rs**
    - **Objective:** Define the subsystem error type and config struct.
    - **Files/Functions to Modify/Create:** `inference/src/error.rs`, `inference/src/config.rs`, `inference/config/inference.toml`, `inference/src/main.rs`
    - **Tests to Write:** `test_config_load_valid`, `test_config_validation_zero_vram`, `test_config_validation_zero_queue`, `test_config_validation_missing_model`, `test_error_to_status_model_not_found`, `test_error_to_status_queue_full`, `test_error_to_status_timeout`, `test_error_to_status_oom`
    - **Steps:**
        1. Write tests in error.rs (fail first)
        2. Define InferenceError with thiserror
        3. Implement From<InferenceError> for tonic::Status
        4. Write config tests
        5. Define Config struct with TOML loading + validation
        6. Write inference/config/inference.toml
        7. Run all tests — confirm pass

3. **Phase 3: model_registry.rs**
    - **Objective:** Model registry tracking load state, VRAM allocation, LRU metadata
    - **Files/Functions to Modify/Create:** `inference/src/model_registry.rs`
    - **Tests to Write:** 7 functional tests for register, state transitions, active model, LRU, VRAM
    - **Steps:** Write tests first, implement ModelLoadState enum, ModelEntry struct, ModelRegistry struct

4. **Phase 4: model_loader.rs**
    - **Objective:** Wrap llama-cpp-rs behind async spawn_blocking functions
    - **Files/Functions to Modify/Create:** `inference/src/model_loader.rs`
    - **Tests to Write:** 4 tests for VRAM estimation, load errors, Send assertion
    - **Steps:** Read embedder.rs patterns, implement ModelHandle, estimate_vram_mb, async load/unload

5. **Phase 5: request_queue.rs**
    - **Objective:** Priority-aware async request queue
    - **Files/Functions to Modify/Create:** `inference/src/request_queue.rs`
    - **Tests to Write:** 6 tests for push/pop, priority ordering, queue full, expiry, blocking pop
    - **Steps:** Implement Priority enum, QueuedRequest, RequestQueue with BinaryHeap + Notify

6. **Phase 6: inference_engine.rs**
    - **Objective:** Core engine with registry, loader, queue, worker loop, OOM recovery
    - **Files/Functions to Modify/Create:** `inference/src/inference_engine.rs`
    - **Tests to Write:** 6 tests for engine init, load, VRAM check, enqueue, switch, OOM recovery
    - **Steps:** Implement InferenceEngine with load/unload/switch/enqueue/run_worker

7. **Phase 7: grpc.rs**
    - **Objective:** InferenceService gRPC trait implementation
    - **Files/Functions to Modify/Create:** `inference/src/grpc.rs`
    - **Tests to Write:** 5 tests for complete, stream_complete, unimplemented stubs, queue full
    - **Steps:** Wire Complete/StreamComplete to engine, stub ReadActivations/Steer

8. **Phase 8: main.rs — Boot Sequence and Shutdown**
    - **Objective:** Wire everything together with boot sequence and graceful shutdown
    - **Files/Functions to Modify/Create:** `inference/src/main.rs`
    - **Tests to Write:** 2 tests for boot gate and shutdown signal
    - **Steps:** Config load → tracing → daemon-bus connect → boot gate → engine init → worker spawn → gRPC server → INFERENCE_READY → shutdown handling

**Open Questions**
1. llama-cpp-rs API surface — read embedder.rs for proven patterns, use FIXME markers for unclear calls
2. Streaming strategy — Option A (buffer then stream) for Phase 1
