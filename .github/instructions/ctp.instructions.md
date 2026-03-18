---
applyTo: "ctp/**"
---

# ctp — Copilot Instructions

CTP (Continuous Thought Processing) is Sena's proactive cognitive loop. It runs continuously at low priority from the moment daemon-bus starts. It never fires on a schedule — it runs always and uses a relevance evaluator to decide what is worth surfacing. It is an extractable open-source subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Language

**Rust only.** All async is tokio. All Win32 activity detection (GetLastInputInfo) is called via the `windows` crate. No Python, no asyncio. Any pseudocode in research or planning documents describing CTP's behavior in Python-like syntax is illustrative only — the implementation is always Rust.

---

## Ownership Boundaries

CTP owns:
- Continuous thought generation from telemetry, memory associations, and behavioral patterns
- Relevance scoring and evaluation of candidate thoughts
- The thought queue — priority, expiry, and surfacing logic
- Prompt context assembly — gathering and packaging all inputs for prompt-composer
- Requesting priority escalation from daemon-bus when needed
- Memory consolidation requests — instructing memory-engine to promote, deprecate, or update entries
- Reading the telemetry and log stream as live inputs
- Win32 activity state detection — polling `GetLastInputInfo` to determine idle thresholds

CTP does not own:
- Prompt assembly itself — that is prompt-composer. CTP assembles the inputs and hands them off
- Memory reads and writes directly — CTP instructs memory-engine via gRPC, never touches memory internals
- Deciding how a prompt is serialized — that is prompt-composer
- Surfacing thoughts to the user directly — that goes through the reactive loop
- SoulBox reads beyond what is needed for relevance weighting — CTP reads SoulBox state, it never writes to it

---

## Core Loop Traps

### CTP Never Sleeps — It Runs or It Is Dead

CTP is not a scheduled task. Never implement CTP as a timer or polling loop with `tokio::time::sleep`. It is a continuous async loop that yields only when awaiting the next signal from the telemetry stream or thought queue.

```rust
// bad — polling with sleep
loop {
    tokio::time::sleep(Duration::from_secs(1)).await;
    evaluate_thoughts().await;
}

// good — continuous loop that yields naturally on stream input
while let Some(signal) = telemetry_stream.next().await {
    let thought = generate_thought(signal).await;
    thought_queue.push(thought).await;
}
```

### Thought Generation Is Never Blocking

Thought generation must never block the evaluation cycle. Use `tokio::spawn` to generate thoughts concurrently with evaluation. A slow thought generation must not stall the queue consumer.

### The Thought Queue Is Priority-Ordered, Never FIFO

The queue orders by relevance score, not insertion order. A high-relevance thought generated late must surface before a low-relevance thought generated earlier. Use `async-priority-queue` or a `BinaryHeap` wrapped in a `Mutex` with a `Notify`.

```rust
// bad — FIFO channel
thought_tx.send(thought).await?;

// good — priority insertion
thought_queue.push(thought, OrderedFloat(thought.relevance_score)).await;
```

### Thoughts Have Expiry — Always Set It

Every thought pushed to the queue must have an expiry timestamp. A thought that has not surfaced within its window is discarded silently. The expiry window is derived from the relevance score — high relevance gets a longer window, low relevance gets a short one. Never hardcode the window duration.

```rust
// bad — no expiry
thought_queue.push(Thought { content, score }, priority).await;

// good — expiry always set, derived from config and score
let expiry = Instant::now() + config.expiry_window(thought.relevance_score);
thought_queue.push(Thought { content, score, expires_at: expiry }, priority).await;
```

---

## Relevance Scoring Traps

### Relevance Weights Come From SoulBox — Never Hardcoded

The weights for urgency, emotional resonance, novelty, recurrence, and idle curiosity are SoulBox properties. Never hardcode them as constants.

```rust
// bad — hardcoded weights
const URGENCY_WEIGHT: f32 = 0.9;
const NOVELTY_WEIGHT: f32 = 0.5;

// good — read from SoulBox state at runtime
let weights = soulbox_snapshot.ctp_relevance_weights();
let score = compute_score(&signals, &weights);
```

### Surface Threshold Is Dynamic — Never Fixed

The threshold for surfacing a thought changes based on user activity state. Never use a fixed threshold.

```rust
// bad — fixed threshold
const SURFACE_THRESHOLD: f32 = 0.7;

// good — dynamic based on activity state
let threshold = activity_state.surface_threshold(); // UserActive=0.9, Idle2Min=0.6, Idle10Min=0.3
```

### Low-Score Thoughts Are Discarded at Trace Level

Never log discarded low-score thoughts above `trace` level. CTP generates thoughts constantly — logging every discard at `debug` or higher creates noise that obscures real signal.

---

## Activity State Traps

### Activity State Uses GetLastInputInfo — No Substitutes

Win32 `GetLastInputInfo` is the correct API for idle detection on Windows. Never poll key/mouse state directly. Never use a timer as a proxy for user activity.

```rust
// good — Win32 idle detection
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

fn idle_duration() -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe { GetLastInputInfo(&mut info) };
    let tick_now = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    Duration::from_millis((tick_now - info.dwTime) as u64)
}
```

### Activity Polling Is Background — Never In the Hot Path

Activity state is polled on a dedicated background task, not inside the thought evaluation loop. The hot path reads a cached `Arc<AtomicU8>` activity state value, never calls Win32 directly.

---

## Escalation Traps

### Never Self-Escalate — Always Request From daemon-bus

CTP cannot change its own priority. It sends an escalation request to daemon-bus via gRPC and waits for a grant before operating at elevated priority.

```rust
// bad — self-elevation
self.priority = Priority::Tier2;

// good — request from arbitrator
let grant = daemon_bus_client
    .request_escalation(EscalationRequest {
        subsystem: SubsystemId::Ctp as i32,
        reason: EscalationReason::DeepMemoryRetrieval as i32,
        max_duration_ms: 5000,
    })
    .await?
    .into_inner();

if grant.approved {
    // perform elevated operation
    // release is automatic via drop of grant token
}
```

### Always Release Escalation — Use RAII Guard

Escalation must always be released when the elevated task completes. Use a RAII guard — never manually track release, it will be missed on error paths.

### Escalation Is Exceptional — Not Routine

If CTP is requesting escalation frequently, that is a design problem. Never build a pattern where CTP escalates on every thought cycle. Escalation is for deep memory retrieval during consolidation only.

---

## Memory Consolidation Traps

### Consolidation Happens During Deep Idle Only

Never trigger heavy memory consolidation while the user is active. Consolidation is a background idle task.

```rust
// bad — consolidates regardless of activity
request_memory_consolidation().await?;

// good — respects activity state
if activity_state.is_deep_idle() {
    request_memory_consolidation().await?;
}
```

### CTP Requests Consolidation — It Does Not Execute It

CTP sends a consolidation instruction to memory-engine via gRPC. It never writes to memory directly.

```rust
// bad — direct memory write
memory_store.promote(entry_id, MemoryTier::LongTerm).await?;

// good — gRPC instruction to memory-engine
memory_client
    .promote(PromotionRequest {
        entry_id: entry_id.to_string(),
        reason: PromotionReason::CtpConsolidation as i32,
    })
    .await?;
```

---

## Prompt Context Assembly Traps

### Assembly Is CTP's Job — Serialization Is prompt-composer's Job

CTP gathers all inputs and packages them into a structured `PromptContext`. prompt-composer receives that struct and handles TOON serialization. Never serialize to TOON inside CTP.

```rust
// bad — CTP serializes
let toon_context = toon_format::encode(&memory_results)?;
prompt_composer_client.build(toon_context).await?;

// good — CTP assembles raw context struct
let context = PromptContext {
    soulbox_snapshot: soulbox_state.snapshot(),
    short_term: recent_memories,
    long_term: retrieved_memories,
    episodic: episodic_entries,
    os_context: current_os_state,
    model_profile: active_model_capabilities.clone(),
    user_intent: inferred_intent,
};
prompt_composer_client.build(context).await?;
```

---

## Logging

Use `tracing` exclusively. Required fields on every CTP log event:

- `event_type` — `thought_generated`, `thought_discarded`, `thought_surfaced`, `escalation_requested`, `consolidation_triggered`
- `relevance_score` — always include on thought events
- `activity_state` — current user activity level

```rust
tracing::trace!(
    event_type = "thought_discarded",
    relevance_score = thought.score,
    threshold = current_threshold,
    activity_state = %activity_state,
    reason = "below_threshold",
);
```

Discarded thoughts: `trace` level.
Surfaced thoughts: `info` level.
Escalation requests and grants: `info` level.
Consolidation cycles: `debug` level.