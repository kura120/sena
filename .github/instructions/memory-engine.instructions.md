---
applyTo: "memory-engine/**"
---

# memory-engine — Copilot Instructions

memory-engine is the Rust concurrent memory system. It owns all memory reads, writes, and tier management. It is never idle — multiple subsystems read and write simultaneously at all times. It owns its own concurrency entirely. daemon-bus never coordinates memory access.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

memory-engine owns:
- All memory reads and writes across all tiers (short-term, long-term, episodic)
- RwLock management per tier
- The internal priority queue for read/write ordering
- Memory weighting and relevance decay
- Broadcasting state change events to daemon-bus after writes complete

memory-engine does not own:
- Deciding what to store — that is the responsibility of the calling subsystem
- Deciding what is relevant — relevance scoring belongs to CTP
- Prompt assembly — that is prompt-composer
- SoulBox state — that is soulbox

If you find yourself writing relevance logic or prompt assembly logic inside memory-engine, stop. memory-engine stores and retrieves. It does not reason.

---

## Concurrency Traps

### RwLock Per Tier — Never One Lock for All
Each memory tier (short-term, long-term, episodic) has its own `RwLock`. Never use a single lock across all tiers — this serializes all memory operations and kills performance.

```rust
// bad — one lock for everything
struct MemoryEngine {
    lock: RwLock<AllTiers>,
}

// good — one lock per tier
struct MemoryEngine {
    short_term: RwLock<ShortTermTier>,
    long_term: RwLock<LongTermTier>,
    episodic: RwLock<EpisodicTier>,
}
```

### Reactive Reads Always Preempt CTP Writes
The internal priority queue must always serve reactive read requests before CTP background writes. Never implement a FIFO queue — priority must be respected.

```rust
// bad — FIFO ignores priority
queue.push_back(request);

// good — priority-aware insertion
match request.priority {
    Priority::Reactive => queue.push_front(request),
    Priority::Background => queue.push_back(request),
}
```

### Never Hold a Write Lock During an Async Await
Holding a write lock across an await point blocks all readers for the duration of the async operation. Always complete the write synchronously within the lock scope.

```rust
// bad — holds write lock across await
let mut guard = self.long_term.write().await;
let result = external_call().await; // lock held here!
guard.insert(key, result);

// good — await outside the lock
let result = external_call().await;
let mut guard = self.long_term.write().await;
guard.insert(key, result);
```

### daemon-bus Is Never Involved in Memory Coordination
daemon-bus receives broadcast events after memory state changes. It never grants locks, coordinates writes, or arbitrates memory access. If you find yourself calling into daemon-bus from inside a lock scope, stop.

---

## Memory Tool Stack

memory-engine integrates three tools. Each has a specific role — never use one where another applies.

| Tool | Role | Wrong use |
|---|---|---|
| Cognee | Vector storage and semantic retrieval — what Sena knows | Never rely on Cognee's graph layer for episodic ordering — use LadybugDB instead |
| LadybugDB | Graph layer — entity relationships, `NEXT_EVENT` edges, episodic sequence ordering | Never use for vector/semantic search |
| SQLite | Telemetry, agent state, SoulBox schema — used internally by Cognee | Never use for vector search or graph traversal |

LanceDB is embedded inside Cognee and is not a direct integration point for memory-engine. Do not import or call LanceDB directly — always go through Cognee's API.

---

## Cognee Integration Traps

These are confirmed failure modes discovered during the Cognee spike. Every trap here has a reproduction case in `spikes/README.md`.

### cognee.add() Is Not Safe for Concurrent Calls
Cognee's `add()` internally calls `reset_dataset_pipeline_run_status()` which does an unguarded SQLite read-modify-write on the pipeline-run table. Concurrent calls race on that row and produce `list index out of range` deep in the pipeline layer. memory-engine's internal write queue serializes all Cognee adds — this is architecturally correct and must never be bypassed.

```rust
// bad — concurrent adds race on SQLite pipeline table
tokio::join!(
    cognee_add(fact_a),
    cognee_add(fact_b),
);

// good — write queue serializes all Cognee calls
write_queue.enqueue(fact_a).await?;
write_queue.enqueue(fact_b).await?;
```

### cognee.search() Is Not Safe for Concurrent Calls
Cognee's `search()` writes results back to a SQLite cache table. Concurrent searches cause simultaneous INSERTs that hit a write lock — SQLite's default journal mode has no WAL. The reactive loop fires one search at a time, which is the correct production pattern. Never parallelize Cognee searches.

### cognify() Background Writes Outlive the Coroutine
`cognify()` returns before all background SQLite writes (token_count updates via SQLAlchemy autoflush) complete. Any Cognee operation immediately following `cognify()` may race against those writes and hit `database is locked`. In production this is not an issue because CTP's write queue naturally spaces operations. Never call `cognify()` and immediately follow it with another Cognee operation in a tight loop.

### Cold Start Must Seed Before Searching
After `prune_data()` + `prune_system()`, Cognee wipes the user and dataset records entirely. A subsequent `search()` call with no prior `add()` throws `SearchPreconditionError: no database/default user found`. memory-engine's cold start boot sequence must always call `add()` + `cognify()` with at least one seed record before opening the reactive loop for searches.

### Never Rely on Cognee's Graph Layer for Episodic Ordering
Cognee's internal graph extraction prompt (as of 0.5.x) does not instruct the LLM to include the `name` field required by its own `KnowledgeGraph` Pydantic schema. Graph extraction fails validation on every generation attempt with local models. A patched prompt is maintained in `.venv` — see `spikes/README.md` for the patch and the upstream PR. Until the fix is merged into Cognee's official release, do not architect any feature that depends on Cognee's graph layer for correctness. Episodic ordering is owned by LadybugDB exclusively.

---

## Memory Tier Traps

### Short-Term Is Volatile — Never Persist It Directly
Short-term memory is session-scoped. Never write short-term memory to disk directly. CTP is responsible for promoting short-term entries to long-term during consolidation. memory-engine executes the promotion when instructed — it does not decide when to promote.

### Episodic Sequences Require Explicit NEXT_EVENT Edges in LadybugDB
Never rely on Cognee's chunker proximity to preserve episodic ordering. If the session-log text is split across chunk boundaries, the before/middle/after steps of a sequence end up in separate chunks with no ordering relationship between them. Episodic sequences must be stored with explicit ordering metadata in LadybugDB:

- Each step is a separate graph node
- Nodes carry `(session_id, step_index)` as a composite key
- Adjacent steps are connected by `NEXT_EVENT` edges
- Cognee stores the raw text for semantic retrieval; LadybugDB stores the ordered structure for sequence reconstruction

```rust
// bad — relying on Cognee chunker to preserve sequence
cognee_add("Step 1 ... Step 2 ... Step 3 ...").await?;

// good — explicit ordering in LadybugDB
for (idx, step) in steps.iter().enumerate() {
    cognee_add(&step.text).await?;
    ladybug_create_node(session_id, idx, &step).await?;
    if idx > 0 {
        ladybug_create_edge(session_id, idx - 1, idx, "NEXT_EVENT").await?;
    }
}
```

### Episodic Writes Are Append-Only
Never mutate or delete an existing episodic memory entry. Episodic memory is a log of what happened. Corrections are new entries, not edits.

```rust
// bad — mutates existing episode
episode.update(new_data);

// good — appends a correction entry
episodic_tier.append(EpisodeEntry::correction(original_id, new_data));
```

### Weight Decay Must Be Bounded
Memory weighting decays over time. The decay function must have a minimum floor — never let a weight reach zero through decay alone. A memory can only be fully removed by an explicit delete operation, never by decay.

```rust
// bad — weight can reach zero
weight *= decay_factor;

// good — floor prevents zero
weight = (weight * decay_factor).max(MIN_WEIGHT_FLOOR);
```

---

## Cognee Write Queue

memory-engine owns an internal async write queue that serializes all Cognee operations. This queue is the single point of contact between memory-engine and Cognee. No subsystem calls Cognee directly — all writes go through the queue.

The write queue enforces:
- Sequential `cognee.add()` calls — never concurrent
- Minimum spacing between operations — mirrors CTP's natural operation spacing
- Retry on transient SQLite lock errors with bounded backoff
- Drain detection after `cognify()` — polls until SQLite accepts a probe write before proceeding

CTP, the reactive loop, and all agents submit write requests to the queue. The queue owns the Cognee session lifecycle.

---

## Event Broadcasting Traps

### Broadcast After Write Completes — Never During
Always release the write lock before broadcasting a state change event to daemon-bus. Never broadcast while holding a lock.

```rust
// bad — broadcasts while holding lock
let mut guard = self.long_term.write().await;
guard.insert(key, value);
self.bus.publish(events::MEMORY_UPDATED, payload).await?; // lock still held

// good — release lock first
{
    let mut guard = self.long_term.write().await;
    guard.insert(key, value);
} // lock released here
self.bus.publish(events::MEMORY_UPDATED, payload).await?;
```

---

## Logging

Use the `tracing` crate exclusively. Required fields on every memory operation log:

- `tier` — which memory tier was accessed
- `operation` — read, write, promote, deprecate
- `priority` — reactive or background
- `duration_ms` — how long the operation took

```rust
tracing::debug!(
    tier = "long_term",
    operation = "write",
    priority = "background",
    duration_ms = elapsed.as_millis(),
    "memory write completed"
);
```

Slow operations (> 100ms) must be logged at `warn` level regardless of priority.