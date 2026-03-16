---
applyTo: "memory-engine/**"
---

# memory-engine — Copilot Instructions

memory-engine is the Sena-specific integration layer that wires ech0 to llama-cpp-rs and daemon-bus. It owns all memory reads, writes, and tier management. It is never idle — multiple subsystems read and write simultaneously. It owns its own concurrency entirely. daemon-bus never coordinates memory access.

The sole backend is ech0 (`Store`, `Embedder`, `Extractor`). There is no Python layer. There is no external process. There is no network call.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

memory-engine owns:
- All memory reads and writes across all tiers (short-term, long-term, episodic)
- RwLock management per tier
- The internal priority write queue
- Memory weighting and relevance decay (via ech0's `importance-decay` feature)
- Broadcasting state change events to daemon-bus after writes complete
- Implementing `Embedder` and `Extractor` traits for ech0 backed by llama-cpp-rs
- Deriving `StoreConfig` from `ModelCapabilityProfile` (profile.rs)

memory-engine does not own:
- Deciding what to store — that is the calling subsystem's responsibility
- Deciding what is relevant — relevance scoring belongs to CTP
- Prompt assembly — that belongs to prompt-composer
- SoulBox state — that belongs to soulbox
- ech0 internals — never reach past the public API (`Store`, traits, schema types)

If you find yourself writing relevance logic or prompt assembly inside memory-engine, stop. memory-engine stores and retrieves. It does not reason.

---

## ech0 Integration

### Entry Point
```rust
use ech0::{Store, StoreConfig};

let store = Store::new(config, embedder, extractor).await?;
```

`Store` is the only ech0 type that memory-engine constructs directly. All other ech0 types (`Node`, `Edge`, `SearchResult`, `IngestResult`, etc.) are read-only outputs — never construct them manually.

### Traits memory-engine Must Implement
```rust
// In embedder.rs — backed by llama-cpp-rs
impl Embedder for LlamaEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EchoError>;
}

// In extractor.rs — backed by llama-cpp-rs structured output
impl Extractor for LlamaExtractor {
    async fn extract(&self, text: &str) -> Result<ExtractionResult, EchoError>;
}
```

Both traits require `Send + Sync`. Both return `EchoError` — not `SenaError`. The conversion to `SenaError` happens at the call site in `engine.rs`, never inside the trait impl.

### Features Enabled in Sena
```toml
ech0 = { path = "../ech0", features = ["dynamic-linking", "importance-decay", "provenance", "contradiction-detection"] }
```

All four features are always enabled. Never conditionalize code on whether these features are present.

### EchoError Is Never Propagated Raw

Every `EchoError` returned by ech0 must be mapped to `SenaError` at the engine.rs call site before it leaves memory-engine's internal layer.
```rust
// bad — propagates EchoError outside engine boundary
pub async fn write(&self, entry: MemoryEntry) -> Result<(), EchoError> { ... }

// good — maps at the boundary
pub async fn write(&self, entry: MemoryEntry, priority: Priority) -> Result<(), SenaError> {
    self.store.ingest_text(&entry.text).await.map_err(SenaError::from)?;
    ...
}
```

---

## Profile Derivation (profile.rs)

memory-engine receives `ModelCapabilityProfile` from daemon-bus at boot. `profile.rs` derives `StoreConfig` from it. This is the only place where capability-conditional logic lives.

Rules:
- `graph_extraction == CapabilityLevel::None` → disable `DynamicLinkingConfig`
- `structured_output == CapabilityLevel::None` → log a `warn`, set `degraded_extractor: bool` flag on `ProfileDerivedConfig` — do not silently proceed as if extraction works
- `pre_rot_threshold` drives `MemoryConfig` context budget
- `reasoning_quality` drives `ContradictionConfig` sensitivity (higher quality = stricter conflict detection)

Returns `ProfileDerivedConfig`, not `StoreConfig` directly. All derived values logged at `debug` level — field names and values only, no content.

---

## Concurrency Traps

### RwLock Per Tier — Never One Lock for All
```rust
// bad
struct MemoryEngine {
    lock: RwLock<AllTiers>,
}

// good
struct MemoryEngine {
    short_term: RwLock<ShortTermTier>,
    long_term: RwLock<LongTermTier>,
    episodic: RwLock<EpisodicTier>,
    store: Arc<Store>,
    queue: Arc<WriteQueue>,
    bus: Arc<DaemonBusClient>,
    config: Arc<Config>,
}
```

### Reactive Reads Always Preempt Background Writes
```rust
// bad — FIFO ignores priority
queue.push_back(request);

// good
match request.priority {
    Priority::Reactive => queue.push_front(request),
    Priority::Background => queue.push_back(request),
}
```

### Never Hold a Write Lock Across an Async Await
```rust
// bad
let mut guard = self.long_term.write().await;
let result = self.store.ingest_text(&text).await?; // lock held here

// good
let result = self.store.ingest_text(&text).await?;
let mut guard = self.long_term.write().await;
guard.insert(key, result);
```

### daemon-bus Is Never Involved in Memory Coordination

daemon-bus receives broadcast events after memory state changes complete. It never grants locks, coordinates writes, or arbitrates memory access. Never call into daemon-bus from inside a lock scope.

---

## Write Queue (queue.rs)

All `store.ingest_text()` calls are serialized through the async write queue in `queue.rs`. No subsystem calls ech0 directly.

The queue enforces:
- Sequential ech0 ingest calls — never concurrent
- Priority ordering — Reactive at front, Background at back
- Max depth from config — returns `ErrorCode::QueueFull` if exceeded
- Per-item bounded timeout (`operation_timeout_ms` from config) — returns `ErrorCode::QueueTimeout`
- Retry on transient `EchoError` — up to `max_attempts` with `backoff_ms` spacing

The drain task `JoinHandle` must be stored — never silently dropped.
```rust
// bad
tokio::join!(store.ingest_text(a), store.ingest_text(b));

// good
write_queue.enqueue(entry_a, Priority::Background).await?;
write_queue.enqueue(entry_b, Priority::Background).await?;
```

---

## Memory Tier Traps

### Short-Term Is Volatile — Never Persist It Directly

Short-term memory is session-scoped. CTP decides when to promote to long-term. memory-engine executes the promotion — it never decides when.

### Episodic Tier Is Append-Only

Never mutate or delete an existing episodic entry. Corrections are new entries. `EpisodicTier` exposes no `update` or `delete` methods — enforced by the type.

### Weight Decay Must Have a Floor
```rust
// bad
weight *= decay_factor;

// good
weight = (weight * decay_factor).max(config.decay.floor);
```

---

## Event Broadcasting

Always release the write lock before broadcasting to daemon-bus.
```rust
// bad — broadcasts while holding lock
let mut guard = self.long_term.write().await;
guard.insert(key, value);
self.bus.publish(TOPIC_MEMORY_WRITE_COMPLETED, payload).await?;

// good
{
    let mut guard = self.long_term.write().await;
    guard.insert(key, value);
}
self.bus.publish(TOPIC_MEMORY_WRITE_COMPLETED, payload).await?;
```

Events:
- `TOPIC_MEMORY_WRITE_COMPLETED` — after every successful write
- `TOPIC_MEMORY_TIER_PROMOTED` — after every short-term → long-term promotion

---

## Error Handling
```rust
struct SenaError {
    code: ErrorCode,
    message: String,
    debug_context: Option<DebugContext>,  // never crosses gRPC boundary
}
```

- `debug_context` always stripped before any gRPC response
- `message` never contains raw text, user messages, or memory content — only operation names and error codes
- No silent failures — every `EchoError` mapped to `SenaError` before leaving engine.rs

ErrorCode variants: `StorageFailure`, `EmbedderFailure`, `ExtractorFailure`, `ProfileMissing`, `ProfileInvalid`, `QueueFull`, `QueueTimeout`, `BootTimeout`, `GrpcFailure`, `ConfigLoadFailure`.

---

## Logging

`tracing` crate only. No `println!`, no `eprintln!`.

Required fields on every memory operation:
- `tier` — short_term, long_term, episodic
- `operation` — read, write, promote
- `priority` — reactive, background
- `duration_ms`

Operations exceeding 100ms logged at `warn` regardless of priority.

Never log: memory entry content, raw ingest text, user messages, model output, SoulBox values.

---

## Boot Sequence
```
1. Load config from memory-engine.toml
2. Initialize tracing
3. Receive ModelCapabilityProfile from daemon-bus
4. Derive ProfileDerivedConfig (profile.rs)
5. Construct LlamaEmbedder and LlamaExtractor
6. Initialize ech0 Store
7. Initialize MemoryEngine
8. Start gRPC server (MemoryService)
9. Signal MEMORY_ENGINE_READY to daemon-bus
10. Await shutdown signal
```

Any failure in steps 1–8 is fatal. Log error code + subsystem name, signal failure to daemon-bus, exit non-zero.