---
applyTo: "daemon-bus/**"
---

# daemon-bus — Copilot Instructions

daemon-bus is the Rust root process. It owns process supervision, the gRPC server, the internal tokio event bus, and priority arbitration. It is the only process that is never restarted by anything else — if daemon-bus goes down, everything goes down.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

daemon-bus owns:
- The gRPC server and all `.proto` definitions in `proto/`
- Process lifecycle for all child subsystems (spawn, supervise, restart)
- Priority tier arbitration — granting and revoking escalations
- The internal tokio broadcast channel event bus
- Aggregation of all subsystem log streams

daemon-bus does not own:
- Memory reads or writes — that is memory-engine
- Agent logic or routing — that is agents/
- Prompt assembly — that is prompt-composer
- Any business logic — daemon-bus is infrastructure only

If you find yourself writing logic that makes decisions about Sena's behavior inside daemon-bus, stop. That logic belongs elsewhere.

---

## Proto File Rules

All `.proto` definitions live in `shared/proto/`. This is the single source of truth for all gRPC contracts across the entire project.

- Never define a proto message or service outside of `shared/proto/`
- Never add a topic, method, or message without updating the proto file first
- Proto changes require implementation changes in all affected subsystems in the same PR — never leave a proto change unimplemented
- Generated code goes in each subsystem's `src/generated/` — never edit generated files manually

---

## Process Supervision Traps

### Restart Policy Is Exact
The retry policy is: immediate → 5s backoff → 30s backoff → degraded mode. Do not invent a different policy. Do not add retries beyond 3.

```rust
// bad — arbitrary backoff
tokio::time::sleep(Duration::from_secs(10)).await;

// good — follows the spec
let backoff = [Duration::ZERO, Duration::from_secs(5), Duration::from_secs(30)];
```

### Never Restart daemon-bus From Within daemon-bus
daemon-bus supervises others. Nothing supervises daemon-bus. Do not write self-restart logic.

### Subsystem State on Restart
When restarting a crashed subsystem, daemon-bus must log the last known state before attempting restart. Never restart blindly without capturing the crash context first.

---

## Priority Arbitration Traps

### Tier 2 Is Exclusive
Never grant Tier 2 to two subsystems simultaneously. The second request must be queued, not granted.

```rust
// bad — grants without checking
fn grant_escalation(&mut self, subsystem: SubsystemId) {
    self.tier_two_holder = Some(subsystem);
}

// good — checks exclusivity first
fn grant_escalation(&mut self, subsystem: SubsystemId) -> EscalationResult {
    if self.tier_two_holder.is_some() {
        self.escalation_queue.push_back(subsystem);
        return EscalationResult::Queued;
    }
    self.tier_two_holder = Some(subsystem);
    EscalationResult::Granted
}
```

### Escalations Must Expire
Every granted escalation must have a bounded expiry. Never grant an escalation without scheduling its expiry via a tokio timer. On expiry, de-escalate automatically without waiting for the subsystem to release.

### Reactive Loop Always Wins Tier 2 Over CTP
If the reactive loop and CTP both request Tier 2 simultaneously, the reactive loop is always granted first. CTP is queued. This is not configurable.

---

## tokio Event Bus Traps

### Topic Strings Come From Proto Constants Only
Never write a topic string as a literal. Always reference the constant generated from the proto definition.

```rust
// bad
bus.publish("user.message.received", payload).await?;

// good
bus.publish(topics::USER_MESSAGE_RECEIVED, payload).await?;
```

### Never Block the Bus
The event bus runs on the tokio runtime. Never perform blocking I/O on the bus thread. Spawn blocking work onto a dedicated thread with `tokio::task::spawn_blocking`.

```rust
// bad — blocks the async runtime
bus.subscribe(topic, |event| {
    std::fs::write(path, data)?; // blocking!
});

// good
bus.subscribe(topic, |event| async move {
    tokio::task::spawn_blocking(move || std::fs::write(path, data)).await??;
});
```

### Dropped Tasks Cancel Work
`tokio::spawn` returns a `JoinHandle`. If it is dropped, the task is cancelled. Store handles for any work that must complete, or detach explicitly with `.detach()` only when cancellation is truly acceptable.

---

## Logging

Use the `tracing` crate exclusively. Every significant daemon-bus event must be logged with structured fields.

Required fields on every log event:
- `subsystem` — which subsystem the event concerns
- `event_type` — what happened
- Any relevant IDs or state

```rust
// bad
tracing::info!("restarting subsystem");

// good
tracing::info!(
    subsystem = %subsystem_id,
    event_type = "restart_attempt",
    attempt = retry_count,
    "subsystem restart initiated"
);
```
