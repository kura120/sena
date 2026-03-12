---
applyTo: "codebase-context/**"
---

# codebase-context — Copilot Instructions

codebase-context is Sena's architectural self-awareness layer. It provides a structured, queryable, read-only index of Sena's own subsystem graph — what exists, what each subsystem does, what its interfaces are, and which subsystems are currently active or degraded. It is generated at build time and kept live at runtime via daemon-bus status events. It is an extractable open-source subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

codebase-context owns:
- The build-time index generation pipeline (`codebase-context/generate.py`)
- The generated `codebase-context/graph.json` — the static structural snapshot
- The runtime status overlay — live `active | degraded | disabled` flags per subsystem
- Exposing the full subsystem graph to consumers via gRPC read interface
- Updating subsystem status flags when daemon-bus broadcasts lifecycle events

codebase-context does not own:
- Any subsystem business logic — it describes, never drives
- Any memory reads or writes — the index is its own artifact
- Any model calls
- Deciding what subsystems should do based on the index — that is the consumer's responsibility
- Writing to or modifying any subsystem it indexes

If you find yourself writing logic that causes codebase-context to affect subsystem behavior, stop. It observes and describes. It never acts.

---

## Index Generation Traps

### The Index Is Generated From Source — Never Hand-Written
`graph.json` is produced by `generate.py` at build time by parsing `.proto` files, subsystem manifests (`manifest.toml` in each subsystem root), and the Copilot instruction files. Never manually edit `graph.json` — it will be overwritten on next build.

### Every Subsystem Must Have a manifest.toml
For `generate.py` to index a subsystem, that subsystem must have a `manifest.toml` at its root. If a new subsystem is added to the project without a manifest, it will not appear in the index. Adding a subsystem requires adding its manifest in the same PR.

```toml
# example: memory-engine/manifest.toml
name = "memory-engine"
language = "rust"
responsibility = "Concurrent tiered memory reads and writes across all tiers"
capability_flags = ["MEMORY_ENGINE_READY"]
```

### The Index Is Committed — Never Gitignored
`graph.json` is a build artifact but it is committed. It serves as the runtime source of truth and must be present when Sena boots. Never add it to `.gitignore`.

---

## Runtime Status Traps

### Status Is Overlay — Never Merged Into graph.json
`graph.json` is the static structural snapshot from build time. Runtime status (`active | degraded | disabled`) is a separate in-memory overlay that is applied on top of the static index at query time. Never write status back into `graph.json`.

```python
# bad — merges status into static file
graph["subsystems"]["memory-engine"]["status"] = "degraded"
json.dump(graph, open("graph.json", "w"))

# good — status lives in the overlay only
self._status_overlay["memory-engine"] = SubsystemStatus.DEGRADED
```

### Status Updates Come From daemon-bus Only
The only source of truth for subsystem status is daemon-bus lifecycle events (`SUBSYSTEM_READY`, `SUBSYSTEM_DEGRADED`, `SUBSYSTEM_DISABLED`). Never infer status from anything else. Never poll subsystems directly.

```python
# good — update on daemon-bus event only
async def on_subsystem_event(self, event: SubsystemLifecycleEvent) -> None:
    self._status_overlay[event.subsystem_id] = event.status
```

### Initial Status Is Unknown — Not Active
At boot, before any readiness signals arrive from daemon-bus, all subsystem statuses are `UNKNOWN`. Never default to `ACTIVE` at boot. Status transitions to `ACTIVE` only when the subsystem's readiness signal is received.

---

## Query Interface Traps

### The Interface Is Read-Only — No Mutations
The gRPC interface exposed by codebase-context has no write methods. Consumers query the graph and receive snapshots. They cannot modify it. If a consumer needs to write architectural metadata, that belongs in the subsystem's own manifest, not through codebase-context.

### Queries Return Snapshots — Not Live References
Every query response is a snapshot of the graph at that moment. Consumers must not hold references to returned objects and assume they stay current — they must re-query if they need the latest status.

### Capability Flags Are Strings From Manifests — Never Invented
Capability flags in the index come directly from each subsystem's `manifest.toml`. Never generate or invent capability flag strings in `generate.py`. If a flag is not declared in a manifest, it does not exist in the index.

---

## SubsystemNode Schema

Every node in the graph follows this structure. Never add fields to the schema without updating both `generate.py` and the proto definition in `daemon-bus/proto/codebase_context.proto`.

```python
@dataclass
class SubsystemNode:
    name: str                        # e.g. "memory-engine"
    language: str                    # rust | python | csharp | freya
    responsibility: str              # one-sentence description from manifest
    interfaces: list[str]            # gRPC service names this subsystem exposes
    dependencies: list[str]          # subsystem names this node waits on at boot
    capability_flags: list[str]      # readiness signals this subsystem emits
    status: SubsystemStatus          # UNKNOWN | ACTIVE | DEGRADED | DISABLED
```

---

## Consumers

These subsystems read from codebase-context. Each consumer has a specific, bounded use case — never expand what a consumer reads beyond what it needs.

| Consumer | What They Read | Why |
|---|---|---|
| model-probe | Full graph + capability flags | Determines which capabilities the model must support |
| lora-manager | Interfaces + data flow paths | Identifies architecturally significant reasoning patterns |
| tacet/reflection | Status + capability surface | Honest self-representation when Sena reasons about herself |
| debug UI | Full live graph with status | Real-time architectural visibility |
| CTP | Capability flags + degraded subsystems | Adjusts thought generation based on available capabilities |

---

## Logging

Use `structlog` for the Python generation pipeline. Use `tracing` for the Rust runtime status layer. Required fields:

- `event_type` — index_generated, status_updated, query_served
- `subsystem` — which subsystem the event concerns (on status events)
- `status` — new status value (on status_updated events)

Index generation must be logged at `info` level with subsystem count and any subsystems skipped due to missing manifests.
Status updates must be logged at `debug` level.
Missing manifest warnings must be logged at `warn` level — a subsystem without a manifest is invisible to Sena's self-model.
