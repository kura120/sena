---
applyTo: "agents/**"
---

# agents — Copilot Instructions

The agents subsystem hosts all of Sena's executor agents and the agent framework that powers them. Agents receive routed tasks, execute them, and return typed results. They do not reason — reasoning belongs to CTP and the Reasoning agent. All built-in agents are implemented in Rust against the `Agent` trait. Community agents compile to `.dll` and load dynamically against the `sena-agent-sdk` crate.

This subsystem also owns `AgentScanner` — the review and security pipeline that gates every community agent before it can run.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Language

**Rust only.** No Python. No Agno. No asyncio. All agents are Rust binaries or dynamic libraries. The agent framework is a thin custom Rust layer — approximately 600 lines of core framework code across three layers:

- `Agent` trait — stable public contract. `manifest()` + `run(task, context)`. Never breaks post-V1.
- `AgentRuntime` — framework internals: `Router`, `ToolLoop`, `SandboxEnforcer`.
- `manifest.toml` — what every agent (built-in and community) declares.

---

## Ownership Boundaries

Agents own:
- Executing routed tasks using available OS integrations and model calls
- Returning structured `AgentResult` to the router
- Subscribing to their relevant daemon-bus gRPC topics only
- Community agent scanning, review, registry management, and quarantine (`AgentScanner`)

Agents do not own:
- Routing decisions that require reasoning — that is CTP
- Memory reads/writes directly — all memory access goes through memory-engine gRPC
- Prompt assembly — that is prompt-composer
- Multi-step planning — that is the Reasoning agent
- SoulBox reads — agents are stateless executors; identity state belongs to tacet

---

## Agent Trait

Every built-in agent implements this trait. Community agents implement the same trait via `sena-agent-sdk`.

```rust
pub trait Agent: Send + Sync {
    fn manifest(&self) -> &AgentManifest;
    async fn run(&self, task: AgentTask, context: AgentContext) -> Result<AgentResult, AgentError>;
}
```

- `manifest()` is called once at registration. Never call it per-task.
- `run()` receives all task state as arguments. Never store task-specific state on `self`.
- `AgentResult` is always typed — never return a raw string.

---

## Agent Framework Traps

### Agents Are Always Warm — Never Spawned On Demand

All agents are instantiated at startup and remain idle-listening. Never create a new agent instance in response to a request.

```rust
// bad — spawns on demand
async fn handle_request(request: AgentTask) -> AgentResult {
    let agent = FileAgent::new();
    agent.run(request, context).await
}

// good — warm instance used from registry
let agent = runtime.get_agent(AgentId::File)?;
agent.run(task, context).await
```

### The Router Never Executes Tasks — Only Delegates

The router's only job is to interpret the request and dispatch to the correct agent(s). Never put execution logic in the router.

### Parallel Dispatch Requires Full Result Collection

When the router dispatches to multiple agents simultaneously, use `tokio::join!` or `futures::join_all` and collect all results before returning. Never return a partial result from parallel dispatch.

```rust
// bad — returns first result only
let result = tokio::select! {
    r = agent_a.run(task_a, ctx.clone()) => r,
    r = agent_b.run(task_b, ctx.clone()) => r,
};

// good — collects all
let (result_a, result_b) = tokio::join!(
    agent_a.run(task_a, ctx.clone()),
    agent_b.run(task_b, ctx.clone()),
);
merge_results(result_a?, result_b?)
```

### Router Falls Back Gracefully on Agent Failure

If a dispatched agent fails, return the best available result from agents that succeeded and include a degraded-mode indicator. Never propagate a raw error to the user.

### Agents Subscribe Only to Their Relevant Topics

Never subscribe to all topics and filter internally.

```rust
// bad
bus.subscribe_all(self.handle_any.clone()).await?;

// good
bus.subscribe(Topic::FileOperationRequested, self.handle_file_op.clone()).await?;
```

### Agent Results Are Always Structured

```rust
// bad
return Ok(AgentResult::raw("file written successfully"));

// good
return Ok(AgentResult {
    agent: AgentId::File,
    status: ResultStatus::Success,
    data: serde_json::json!({ "path": written_path, "bytes": written_bytes }),
    duration_ms: elapsed.as_millis() as u64,
});
```

---

## Memory Access Traps

### All Memory Access Goes Through memory-engine gRPC

No agent reads from or writes to ech0 or memory-engine directly. All memory operations are gRPC calls to `MemoryService`.

```rust
// bad — direct ech0 access
store.ingest_text(content, metadata).await?;

// good — gRPC to memory-engine
memory_client.write(MemoryWriteRequest {
    tier: MemoryTier::ShortTerm,
    content,
    metadata,
    priority: Priority::Standard,
}).await?;
```

### Memory Requests Are Always Typed

Never send a raw string query. Always use a typed request message.

---

## OS Agent Traps

### OS Agents Validate All Inputs Before Execution

Every OS agent must validate its inputs before executing any system operation. Never pass unvalidated user input directly to a file path, process name, or system call.

```rust
// bad
tokio::fs::remove_file(&request.path).await?;

// good
let path = validate_and_canonicalize_path(&request.path)?;
tokio::fs::remove_file(&path).await?;
```

### OS Agents Always Close Handles After Use

Never hold file handles, process handles, or browser connections open between requests. Use RAII — drop handles at the end of the scope that opened them.

### OS Agents Always Report What They Did

Every OS operation must produce a structured result describing exactly what was executed — path, process ID, bytes written, etc. Never return a generic success response.

---

## Concurrency Rules

Each agent instance must be stateless between requests — never store request-specific state in instance fields. All request state is local to the `run()` call.

```rust
// bad — stores request state on self
async fn run(&self, task: AgentTask, ctx: AgentContext) -> Result<AgentResult, AgentError> {
    self.current_path = task.path.clone(); // not safe for concurrent calls
    self.process().await
}

// good — request state is local
async fn run(&self, task: AgentTask, ctx: AgentContext) -> Result<AgentResult, AgentError> {
    let path = task.path.clone();
    self.process(path).await
}
```

---

## AgentScanner — Community Agent Security Pipeline

`AgentScanner` lives inside the `agents/` crate. It owns the full four-layer review pipeline that gates every community agent before it can run. No agent reaches the runtime without passing through the scanner.

### Agent Install Directory

Community agents are installed by the user to:

```
~/.sena/agents/
├── pending/       ← detected but not yet reviewed
│   └── my-agent/
│       ├── agent.dll
│       └── manifest.toml
├── approved/      ← registered, reviewed, cleared
│   └── my-agent/
│       ├── agent.dll
│       └── manifest.toml
└── quarantine/    ← rejected or flagged
    └── my-agent/
        ├── agent.dll
        └── manifest.toml
```

The runtime only loads agents from `approved/`. `AgentScanner` moves agents between directories based on review outcome. Never load from `pending/` or `quarantine/` under any condition.

### Detection — Two Triggers

**Runtime detection:** A filesystem watcher monitors `~/.sena/agents/pending/`. Any new `.dll` dropped there immediately triggers the scanner pipeline if Sena is running.

**Boot-time detection:** On startup, `AgentScanner` scans `~/.sena/agents/pending/` for any agents that arrived while Sena was offline. Each one runs the full pipeline before agents signals `AGENTS_READY`.

### The Four-Layer Pipeline

All four layers run in order before the user sees any approval prompt. Never skip a layer. Never re-order them.

**Layer 1 — Manifest parsing**
Read `manifest.toml`. Validate the schema. Extract all declared permissions. If the manifest is malformed or missing, move to `quarantine/` immediately — do not proceed.

**Layer 2 — Static binary analysis (PE import table)**
Before the user sees anything, `AgentScanner` inspects the `.dll`:

- Parse the PE import table — what Windows APIs does the DLL actually import?
- Cross-reference imports against declared permissions. A mismatch (e.g. DLL imports `WriteFile` but only declares `file.read`) is a hard flag.
- Check for known dangerous imports: `CreateRemoteThread`, `VirtualAllocEx`, `SetWindowsHookEx`, `NtQuerySystemInformation`, and similar injection/surveillance APIs. Any match → automatic quarantine, no user prompt.
- String scan for hardcoded URLs, IP addresses, known C2 patterns, and encoded payloads.

If Layer 2 flags the agent, move to `quarantine/`, notify the user with the specific reason, and stop. Do not show the approval prompt.

**Layer 3 — User review prompt**
Surface the full manifest permission list to the user. Dangerous permissions (per PRD §3 Community Agent Permission Model) require explicit per-permission acknowledgment. The user must approve each dangerous permission individually — a single "allow all" is not acceptable.

If the user rejects, move to `quarantine/`. If the user approves, proceed.

**Layer 4 — Registry entry**
Write a signed entry to `~/.sena/agent-registry.toml`. This is a separate encrypted file (Argon2id + AES-256-GCM, same key derivation as SoulBox, separate file and encryption context). Move the agent from `pending/` to `approved/`. Emit `TOPIC_AGENT_REGISTERED` on daemon-bus.

### Registry Separation Rule

The agent registry is **not** a SoulBox record. It is a separate encrypted TOML file with its own encryption context and failure domain. SoulBox corruption must never touch agent registry state, and agent registry corruption must never touch SoulBox state.

### Quarantine Behavior

Quarantined agents are blocked but not deleted. The user can re-initiate review or manually delete from `quarantine/`. Never auto-delete a quarantined agent.

### Runtime Behavioral Monitoring (Layer 4 — Phase 2)

CTP reads the telemetry stream. If an approved agent exhibits patterns inconsistent with its declared manifest at runtime — file I/O volume inconsistent with declared permissions, network attempts, unexpected process spawning — CTP flags it and daemon-bus suspends the agent pending user review. This layer is Phase 2 work. Do not implement in Phase 1.

### AgentScanner Traps

- Never load or execute any code from an agent before Layers 1 and 2 complete
- Never show the user an approval prompt before Layer 2 completes
- Never write a registry entry before the user has completed Layer 3 approval
- Never move an agent to `approved/` without a corresponding registry entry
- The scanner result is always logged with the full analysis output regardless of outcome

---

## Logging

Use `tracing` exclusively. Required fields on every agent log event:

- `agent` — which agent handled the event
- `event_type` — `task_received`, `task_completed`, `task_failed`, `dispatched_to`, `agent_registered`, `agent_quarantined`
- `task_id` — unique ID for the request, for tracing parallel dispatches
- `duration_ms` — how long the task took

```rust
tracing::info!(
    agent = "file",
    event_type = "task_completed",
    task_id = %task.id,
    duration_ms = elapsed.as_millis(),
    status = "success",
);
```

Failed tasks must be logged at `error` level with the full error context. Scanner layer results must be logged at `info` level for approved agents and `warn` level for quarantined agents, with the specific flag reason included.