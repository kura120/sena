# PLAN — memory-engine gRPC server wiring

## What

Fix 7 specific gaps in memory-engine's gRPC server layer so that MemoryService is defined in the proto, generated into both daemon-bus and memory-engine, and served by a real tonic gRPC server with full Write/Read/Promote RPC implementations.

## Why

The memory-engine internals are complete and tested (69 tests passing), but the gRPC server layer is broken or absent. The proto has no MemoryService definition, the generated code has no MemoryService stubs, main.rs runs a plain TCP listener instead of a tonic server, and grpc.rs uses manual placeholder types instead of proto-generated types. These 7 fixes close the gap between the working engine and the daemon-bus integration contract.

## Subsystems Affected

1. **daemon-bus** — proto source file only (sena.daemonbus.v1.proto), plus regenerated code in src/generated/
2. **memory-engine** — config TOML, config.rs, engine.rs, profile.rs, main.rs, grpc.rs, plus regenerated code in src/generated/

No other subsystems are modified.

## Assumptions

1. **ech0 v0.1.0 API is the source of truth.** `SearchResult` has fields `nodes: Vec<ScoredNode>`, `edges: Vec<ScoredEdge>`, `retrieval_path: Vec<RetrievalStep>`. `ScoredNode` has `node: Node`, `score: f32`, `source: RetrievalSource`. `Node` has `id: Uuid`, `kind: String`, `metadata: serde_json::Value`, `importance: f32`, `source_text: Option<String>` — no `summary` or `tier` field. `SearchOptions` has `limit`, `vector_weight`, `graph_weight`, `min_importance`, `tiers` — no `min_score` field.
2. **protoc is available** on the build machine (verified).
3. **No buf snapshot exists** — `buf breaking` cannot be run until one is created. This is noted but not blocking.
4. **OLLAMA_READY field 5 is not in use** by any subsystem — safe to reserve.
5. **Phase 1 does not wire the LlamaExtractor inference loop** — DegradedExtractor is always used regardless of model capability. This is an honest gate, not a bug.
6. **The 69 existing tests do not exercise the gRPC layer** — they test engine, tier, queue, config, profile, and error internals. The grpc.rs tests use the old manual types and will be replaced.

## Fixes in Order

1. **Proto** — reserve OLLAMA_READY, add EventTopic values 41/42, add MemoryService messages + service definition, regenerate both copies
2. **Config TOML** — add `embedding_dim = 768` to `[embedder]`
3. **config.rs** — add `embedding_dim: usize` to `EmbedderConfig`, update test TOML
4. **engine.rs** — change `write()` return to `SenaResult<String>`, fix event topic constants, remove TODO(proto)
5. **profile.rs** — force `degraded_extractor = true` in Phase 1, comment out capability check with `// Phase 2:` prefix
6. **main.rs** — replace hardcoded 768 with `config.embedder.embedding_dim`, replace TCP listener with real tonic server, remove TODO comments
7. **grpc.rs** — full rewrite implementing the proto-generated `MemoryService` trait with Write/Read/Promote RPCs

## Out of Scope

- Implementing the LlamaExtractor inference loop (Phase 2)
- Creating a buf snapshot baseline
- Modifying any file not listed in the 7 fixes
- Changing ech0's API or any other crate