# PLAN — ech0 API alignment fixes

## What

Fix all 72 compilation errors and 1 warning in memory-engine caused by incorrect usage of ech0's public API and llama-cpp-2's API.

## Why

memory-engine was scaffolded before ech0 was implemented. Now that ech0 v0.1.0 is published, the actual API differs from the assumptions made during scaffolding. Every `Store`, `EchoError`, `ErrorCode`, `Embedder`, `Extractor`, `ExtractionResult`, `SearchOptions`, `StoreConfig`, and `LlamaModel` usage must be updated to match the real signatures.

## Subsystems Affected

1. **memory-engine** — all source files touched, primary target of this work

No other subsystems are modified. ech0 and llama-cpp-2 are consumed as dependencies — their code is not changed.

## Assumptions

1. **ech0 v0.1.0 is the source of truth for all API shapes.** Verified by reading the checked-out source at `~/.cargo/git/checkouts/ech0-31079b32eefcc7fd/840dd7e/src/`. All type signatures, field names, and trait definitions are taken from there — not from memory or prior scaffolding assumptions.

2. **`Store<E, X>` is generic over concrete `Embedder` and `Extractor` types.** ech0 does not support `Box<dyn Embedder>` or `Box<dyn Extractor>`. The existing `run_with_store<E, X>` pattern in main.rs is the correct approach — no dynamic dispatch needed.

3. **`EchoError` has three fields: `code: ErrorCode`, `message: String`, `context: Option<ErrorContext>`.** The scaffolding was constructing `EchoError` with struct literal syntax using wrong field names and missing `message`. The convenience constructors (`EchoError::embedder_failure()`, `EchoError::extractor_failure()`) are the correct way to construct errors, with `.with_context()` for optional debug context.

4. **`ErrorCode` variants in ech0 are: `StorageFailure`, `EmbedderFailure`, `ExtractorFailure`, `ConsistencyError`, `ConflictUnresolved`, `InvalidInput`, `CapacityExceeded`.** There is no `EmbeddingFailed` or `ExtractionFailed` variant. The scaffolding used wrong variant names.

5. **`ExtractionResult` has no `empty()` constructor.** Construct it directly with `ExtractionResult { nodes: Vec::new(), edges: Vec::new() }`.

6. **`SearchOptions` uses `limit` not `max_results`.** The `max_results` field name was a scaffolding assumption.

7. **`SearchResult` does not implement `serde::Serialize`.** Cannot use `serde_json::to_string()` on it. Must manually construct the response JSON or extract fields individually.

8. **`StoreConfig` has top-level fields: `store: StorePathConfig`, `memory: MemoryConfig`, `dynamic_linking: DynamicLinkingConfig`, `contradiction: ContradictionConfig`.** The scaffolding used wrong field names (`paths`, `linking`) and tried to set fields that don't exist on the structs (`enabled` on `DynamicLinkingConfig`, `sensitivity` on `ContradictionConfig`, `context_budget` on `MemoryConfig`).

9. **`LlamaModel::load_from_file` takes `(&LlamaBackend, impl AsRef<Path>, &LlamaModelParams)`.** The scaffolding had the arguments in the wrong order and wrong types.

10. **`LlamaModel::new_context` takes `(&LlamaBackend, LlamaContextParams)` — two arguments, not one.** The scaffolding was passing only `&context_params`.

11. **`ech0::Embedder` trait requires both `embed()` and `dimensions()` methods.** The scaffolding was missing the `dimensions()` implementation. The `#[async_trait]` macro on ech0's trait definition handles lifetime desugaring — our impl must also use `#[async_trait]`.

12. **The `context` field on `EchoError` is `Option<ErrorContext>`, not `ErrorContext`.** All struct literal constructions must wrap in `Some(...)`.

13. **The LlamaExtractor TODO is acceptable.** The extractor inference loop (feed tokens, sample, decode, parse structured output) is deferred to Phase 2 per PRD §6.5 ("ech0 — Memory Architecture") — the extraction pipeline requires structured output prompting which is a Phase 2 capability. The `DegradedExtractor` is the V1 fallback path. The existing TODO in extractor.rs documents this correctly.

14. **The gRPC server placeholder TODO is acceptable.** PRD §13.10 notes that proto definitions stabilize incrementally. The MemoryService proto message definition is a follow-up task that does not block the V1 boot sequence — the TCP listener placeholder keeps the port reserved.

15. **The embedding dimensions TODO in main.rs is acceptable.** PRD §6.2 (ModelProbe) specifies that model metadata (including embedding dimensions) comes from the runtime capability profile. Until model-probe delivers dimension info, the hardcoded 768 default is the correct interim approach, documented with a TODO referencing the model-probe integration.

## Changes by File

### `src/main.rs`
- Remove unused `CapabilityLevel` import
- Remove `Box<dyn Embedder>` / `Box<dyn Extractor>` usage (already correct — `run_with_store` is generic)
- Fix `build_store_config`: use correct `StoreConfig` field names and structure
- Fix `connect_to_daemon_bus`: convert `tonic::transport::Error` to `SenaError` instead of `Status`

### `src/embedder.rs`
- Fix `LlamaModel::load_from_file` call: `(&backend, &model_path, &params)`
- Fix `model.new_context`: pass `(&backend, context_params)` — two args
- Replace all `EchoError { code: ..., context: ... }` struct literals with convenience constructors + `.with_context()`
- Add `fn dimensions(&self) -> usize` to the `Embedder` impl

### `src/extractor.rs`
- Fix `LlamaModel::load_from_file` call: `(&backend, &model_path, &params)`
- Fix `model.new_context`: pass `(&backend, context_params)` — two args
- Replace all `EchoError { code: ..., context: ... }` struct literals with convenience constructors + `.with_context()`
- Replace `ExtractionResult::empty()` with `ExtractionResult { nodes: Vec::new(), edges: Vec::new() }`

### `src/error.rs`
- Replace `ech0::ErrorCode::EmbeddingFailed` with `ech0::ErrorCode::EmbedderFailure`
- Replace `ech0::ErrorCode::ExtractionFailed` with `ech0::ErrorCode::ExtractorFailure`
- Fix `Option<ErrorContext>` display formatting (use `{:?}` or match on Some/None)

### `src/grpc.rs`
- Replace `max_results` with `limit` in `SearchOptions` construction
- Remove `serde_json::to_string(&search_result)` — manually construct response JSON

### `src/engine.rs`
- All `Store` references become `Store<E, X>` with proper generic parameters (already correct in the generic impl blocks — verify struct definition)
- Add explicit type annotation on `search()` error closure

### `src/queue.rs`
- All `Store` references in function signatures become `Store<E, X>`
- Add explicit type annotation on `ingest_text` result

## Out of Scope

- Implementing the LlamaExtractor inference loop (Phase 2 — PRD §6.5)
- Adding MemoryService to the daemon-bus proto file (proto stabilization follow-up)
- Adding TOPIC_MEMORY_WRITE_COMPLETED / TOPIC_MEMORY_TIER_PROMOTED to EventTopic enum (proto follow-up)
- Changing ech0's API or adding serde derives to ech0 types
- Modifying any subsystem other than memory-engine