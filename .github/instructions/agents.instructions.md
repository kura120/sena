---
applyTo: "agents/**"
---

# agents — Copilot Instructions

The agents subsystem hosts all of Sena's executor agents. Agents receive routed tasks, execute them, and return results. They do not reason — reasoning belongs to CTP and the Reasoning agent. All agents are orchestrated via Agno and are always warm — booted at startup and idle-listening via gRPC subscriptions.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

Agents own:
- Executing routed tasks using available OS integrations and model calls
- Returning structured results to the router
- Registering their capabilities with Agno on startup
- Subscribing to their relevant gRPC event topics only

Agents do not own:
- Routing decisions — that is the router agent
- Memory reads/writes directly — all memory access goes through the memory agent
- Prompt assembly — that is prompt-composer
- Reasoning or multi-step planning — that is the Reasoning agent
- Any SoulBox reads — agents are stateless executors

---

## Agno-Specific Traps

### Agents Are Always Warm — Never Spawned On Demand
All agents are instantiated at startup and remain idle-listening. Never create a new agent instance in response to a request.

```python
# bad — spawns on demand
async def handle_request(request):
    agent = FileAgent()
    return await agent.run(request)

# good — warm instance used
class FileAgent:
    def __init__(self):
        self.agent = Agent(
            model=ollama_model,
            tools=self.file_tools(),
        )

    async def run(self, request):
        return await self.agent.run(request.task)
```

### Tools Are Registered at Startup — Never Dynamically Added
Every agent's tool set is fixed at instantiation. Never add or remove tools from a running agent.

### Agents Subscribe Only to Their Relevant Topics
Each agent subscribes only to the gRPC event topics it needs. Never subscribe an agent to all events and filter internally — this wakes agents unnecessarily and wastes resources.

```python
# bad — subscribes to everything
await bus.subscribe("*", self.handle)

# good — subscribes to specific topics
await bus.subscribe(topics.FILE_OPERATION_REQUESTED, self.handle)
```

### Agent Results Are Always Structured
Never return a raw string from an agent. Always return a typed result object. The router and caller depend on structured results for merging parallel results.

```python
# bad
return "file written successfully"

# good
return AgentResult(
    agent=AgentId.FILE,
    status=ResultStatus.SUCCESS,
    data={"path": written_path, "bytes": written_bytes}
)
```

---

## Router Agent Traps

### The Router Never Executes Tasks — Only Delegates
The router agent's only job is to interpret the request and dispatch to the correct agent(s). Never put execution logic in the router.

### Parallel Dispatch Requires Result Collection
When the router dispatches to multiple agents simultaneously, it must use `asyncio.gather()` and collect all results before returning. Never return a partial result from parallel dispatch.

```python
# bad — returns first result
result = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)

# good — collects all
results = await asyncio.gather(*[agent.run(task) for agent, task in dispatches])
return merge_results(results)
```

### Router Falls Back Gracefully on Agent Failure
If a dispatched agent fails, the router must handle degraded mode gracefully — return the best available result from agents that succeeded and note what failed. Never propagate a raw exception to the user.

---

## Memory Agent Traps

### All Memory Access Goes Through the Memory Agent
No other agent reads from or writes to memory-engine directly. All memory operations are requests sent to the memory agent, which interfaces with memory-engine via gRPC.

### Memory Agent Requests Are Always Typed
Never send a raw string query to the memory agent. Always use a typed memory request.

```python
# bad
await memory_agent.run("get recent memories about work")

# good
await memory_agent.retrieve(MemoryRequest(
    tier=MemoryTier.LONG_TERM,
    query=query_embedding,
    limit=10,
    min_relevance=0.6
))
```

---

## OS Agent Traps (File, Screen, Process, Browser, Peripheral)

### OS Agents Validate All Inputs Before Execution
Every OS agent must validate its inputs before executing any system operation. Never pass unvalidated user input directly to a file path, process name, or system call.

```python
# bad — unsanitized path
await os.remove(request.path)

# good — validated first
path = validate_path(request.path)
await os.remove(path)
```

### OS Agents Always Report What They Did
Every OS operation must produce a structured result that describes exactly what was done — path, process ID, bytes written, etc. Never return a generic success response.

### OS Agents Never Hold Handles Open
Always close file handles, process handles, and browser connections after use. Never hold OS resources open between requests.

```python
# good
async with aiofiles.open(path, 'r') as file:
    content = await file.read()
# handle closed automatically
```

---

## Concurrency Rules

Multiple agents may run simultaneously via `asyncio.gather()`. Each agent instance must be stateless between requests — never store request-specific state in instance variables.

```python
# bad — stores request state on instance
class FileAgent:
    async def run(self, request):
        self.current_path = request.path  # not safe for concurrent calls
        return await self._process()

# good — request state is local
class FileAgent:
    async def run(self, request):
        path = request.path  # local to this call
        return await self._process(path)
```

---

## Logging

Use `structlog` exclusively. Required fields on every agent log event:

- `agent` — which agent handled the event
- `event_type` — task_received, task_completed, task_failed, dispatched_to
- `task_id` — unique ID for the request, for tracing parallel dispatches
- `duration_ms` — how long the task took

```python
logger.info(
    "task_completed",
    agent="file",
    task_id=request.task_id,
    duration_ms=elapsed,
    status="success"
)
```

Failed tasks must be logged at `error` level with the full exception context.
