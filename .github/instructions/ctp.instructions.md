---
applyTo: "ctp/**"
---

# ctp — Copilot Instructions

CTP (Continuous Thought Processing) is Sena's proactive cognitive loop. It runs continuously at low priority from the moment daemon-bus starts. It never fires on a schedule — it runs always and uses a relevance evaluator to decide what is worth surfacing. It is an extractable open-source subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

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

CTP does not own:
- Prompt assembly itself — that is prompt-composer. CTP assembles the inputs and hands them off
- Memory reads and writes directly — CTP instructs memory-engine via gRPC, never touches memory internals
- Deciding how a prompt is serialized — that is prompt-composer
- Surfacing thoughts to the user directly — that goes through the reactive loop
- SoulBox reads beyond what is needed for relevance weighting — CTP reads SoulBox state, it never writes to it

---

## Core Loop Traps

### CTP Never Sleeps — It Runs or It Is Dead
CTP is not a scheduled task. Never implement CTP as a timer, cron, or polling loop. It is a continuous async loop that yields only when the thought queue is empty or all pending thoughts are below threshold.

```python
# bad — polling with sleep
while True:
    await asyncio.sleep(1.0)
    await evaluate_thoughts()

# good — continuous loop that yields naturally
async def run():
    async for signal in telemetry_stream():
        thought = await generate_thought(signal)
        await thought_queue.push(thought)
```

### Thought Generation Is Never Blocking
Thought generation must never block the evaluation cycle. Generate and evaluate are separate concerns on separate async paths. A slow thought generation must not stall the queue.

### The Thought Queue Is Priority-Ordered, Never FIFO
The thought queue orders by relevance score, not insertion order. A high-relevance thought generated late must surface before a low-relevance thought generated earlier.

```python
# bad — FIFO append
thought_queue.append(thought)

# good — priority insertion
await thought_queue.push(thought, priority=thought.relevance_score)
```

### Thoughts Have Expiry — Always Set It
Every thought pushed to the queue must have an expiry timestamp. A thought that has not surfaced within its window is discarded silently. Never accumulate stale thoughts.

```python
# bad — no expiry
await thought_queue.push(Thought(content=..., score=...))

# good — expiry always set
await thought_queue.push(Thought(
    content=thought_content,
    score=relevance_score,
    expires_at=utcnow() + expiry_window(relevance_score)
))
```

---

## Relevance Scoring Traps

### Relevance Weights Come From SoulBox — Never Hardcoded
The weights for urgency, emotional resonance, novelty, recurrence, and idle curiosity are SoulBox properties. Never hardcode them as constants in CTP.

```python
# bad — hardcoded weights
URGENCY_WEIGHT = 0.9
NOVELTY_WEIGHT = 0.5

# good — read from SoulBox state at runtime
weights = soulbox_state.ctp_relevance_weights
score = compute_score(signals, weights)
```

### Surface Threshold Is Dynamic — Never Fixed
The threshold for surfacing a thought changes based on user activity state. Never use a fixed threshold.

```python
# bad — fixed threshold
SURFACE_THRESHOLD = 0.7

# good — dynamic based on activity
threshold = activity_state.surface_threshold()
# UserActive -> 0.9, Idle2Min -> 0.6, Idle10Min -> 0.3
```

### Low-Score Thoughts Are Discarded Silently
Never log discarded low-score thoughts at anything above `trace` level. CTP generates thoughts constantly — logging every discard creates noise that obscures real signal.

---

## Escalation Traps

### Never Self-Escalate — Always Request From daemon-bus
CTP cannot change its own priority. It must send an escalation request to daemon-bus and wait for a grant before operating at elevated priority.

```python
# bad — self-elevation
self.priority = Priority.TIER_2

# good — request from arbitrator
grant = await daemon_bus.request_escalation(
    subsystem=SubsystemId.CTP,
    reason=EscalationReason.DEEP_MEMORY_RETRIEVAL,
    max_duration_ms=5000
)
if grant.approved:
    async with escalated_context(grant):
        await deep_retrieval()
```

### Always Release Escalation — Use Context Manager
Escalation must always be released when the elevated task completes. Always use a context manager pattern — never manually track release, it will be missed on exceptions.

### Escalation Is Exceptional — Not Routine
If CTP is requesting escalation frequently, that is a design problem. Escalation should be rare. Never build a pattern where CTP escalates on every thought cycle.

---

## Memory Consolidation Traps

### Consolidation Happens During Low Activity Only
Never trigger heavy memory consolidation while the user is active. Consolidation is a background idle task — it runs when the activity state is idle and CTP is in deep reflection mode.

```python
# bad — consolidates regardless of activity
await consolidate_memory()

# good — respects activity state
if activity_state.is_deep_idle():
    await request_memory_consolidation()
```

### CTP Requests Consolidation — It Does Not Execute It
CTP sends a consolidation instruction to memory-engine via gRPC. It never writes to memory directly.

```python
# bad — direct memory write
await memory_engine.promote(entry_id, tier=MemoryTier.LONG_TERM)

# good — instruction via gRPC
await memory_agent.request_promotion(
    entry_id=entry_id,
    reason=PromotionReason.CTP_CONSOLIDATION
)
```

---

## Prompt Context Assembly Traps

### Assembly Is CTP's Job — Serialization Is PC's Job
CTP gathers all inputs and packages them into a structured context object. prompt-composer receives that object and handles TOON serialization and final prompt construction. Never serialize to TOON inside CTP.

```python
# bad — CTP serializes
toon_context = toon_encode(memory_results)
await prompt_composer.build(toon_context)

# good — CTP assembles raw context
context = PromptContext(
    soulbox_snapshot=soulbox_state,
    short_term=recent_memories,
    long_term=retrieved_memories,
    episodic=episodic_entries,
    os_context=current_os_state,
    model_profile=active_model_capabilities,
    user_intent=inferred_intent
)
await prompt_composer.build(context)
```

---

## Logging

Use `structlog` exclusively. Required fields on every CTP log event:

- `event_type` — thought_generated, thought_discarded, thought_surfaced, escalation_requested, consolidation_triggered
- `relevance_score` — always include on thought events
- `activity_state` — current user activity level

```python
# good
logger.debug(
    "thought_discarded",
    relevance_score=thought.score,
    threshold=current_threshold,
    activity_state=activity_state.name,
    reason="below_threshold"
)
```

Discarded thoughts: `trace` level.
Surfaced thoughts: `info` level.
Escalation requests: `info` level.
Escalation grants: `info` level.
Consolidation cycles: `debug` level.
