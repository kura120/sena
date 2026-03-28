# Sena — Architecture

This document is law for structural and organizational decisions. Agents and developers must read this before touching any file structure.

## 1. Guiding Principle

Every subsystem owns exactly one domain. Every cross-domain communication goes through gRPC. No exceptions.

## 2. Repository Structure

````
sena/
├── daemon-bus/                # LAYER: Infrastructure — process supervision, gRPC, event bus
├── inference/                 # LAYER: Cognitive — model execution (llama-cpp-rs)
├── memory-engine/             # LAYER: Cognitive — tiered knowledge storage (ech0)
├── model-probe/               # LAYER: Cognitive — runtime capability detection
├── ctp/                       # LAYER: Cognitive — continuous thought processing
│
├── prompt-composer/           # LAYER: Composition — prompt assembly, TOON encoding
├── reactive-loop/             # LAYER: Composition — user message routing, agent dispatch
│
├── soulbox/                   # LAYER: Identity — personality state, encryption, migrations
├── lora-manager/              # LAYER: Identity — idle-time LoRA training (Python)
│
├── agents/                    # LAYER: Agency — OS task execution
│   ├── router/
│   ├── memory/
│   ├── file/
│   ├── screen/
│   ├── process/
│   ├── browser/
│   ├── peripheral/
│   ├── reasoning/
│   └── tacet/
│
├── platform/                  # LAYER: OS — platform-specific hooks
│   ├── windows/               # C#/.NET (V1)
│   ├── macos/                 # Swift (future)
│   └── linux/                 # Rust (future)
│
├── ui/                        # LAYER: Interface — Tauri v2 overlay (React + TypeScript)
│
├── shared/                    # Shared contracts
│   └── proto/                 # Source of truth for all gRPC proto definitions
│
├── docs/                      # Project governance
│   ├── PRD.md                 # Vision only
│   ├── ARCHITECTURE.md        # This file — structural law
│   ├── PLAN.md                # Active roadmap and milestone tracker
│   ├── AI_RULES.md            # Non-negotiable development rules
│   ├── decisions/             # Architectural Decision Records (ADRs)
│   └── audit/                 # Subsystem audit reports
│
└── .github/
    ├── copilot-instructions.md
    ├── agents/
    │   └── memory/
    │       ├── _global/       # Global architecture decisions
    │       ├── daemon-bus/    # Per-subsystem memory
    │       ├── inference/
    │       ├── memory-engine/
    │       ├── model-probe/
    │       ├── ctp/
    │       ├── prompt-composer/
    │       ├── reactive-loop/
    │       ├── soulbox/
    │       ├── lora-manager/
    │       ├── agents/
    │       ├── platform/
    │       └── ui/
    └── instructions/
        └── [subsystem].instructions.md
````

## 3. Layer Definitions

### Infrastructure Layer — Process supervision, IPC, event routing

- **Owner:** daemon-bus
- **Rule:** No business logic. Infrastructure only.
- daemon-bus is the root process. It starts all other subsystems, manages their lifecycle, arbitrates priority, and routes events. It never reasons about content — only about process health and coordination.

### Cognitive Layer — How Sena thinks and knows

- **Subsystems:** inference, memory-engine, model-probe, ctp
- **Rule:** No UI interaction. No OS calls. Pure computation and storage.
- inference owns all model execution via llama-cpp-rs. memory-engine owns all knowledge storage via ech0. model-probe detects what the active model can do. ctp generates continuous background thoughts.

### Composition Layer — How thoughts become responses

- **Subsystems:** prompt-composer, reactive-loop
- **Rule:** Assembles and routes. Does not originate thoughts.
- reactive-loop handles all user-facing message flow. prompt-composer assembles dynamic prompts using TOON encoding. Neither subsystem generates original reasoning — they orchestrate the cognitive layer's outputs.

### Identity Layer — Who Sena is

- **Subsystems:** soulbox, lora-manager
- **Rule:** State management only. Never calls agents directly.
- soulbox stores encrypted personality and identity state. lora-manager trains reasoning adapters during idle time. Identity data flows outward through gRPC reads — never through direct calls.

### Agency Layer — What Sena can do

- **Subsystems:** agents/ (router, memory, file, screen, process, browser, peripheral, reasoning, tacet)
- **Rule:** Executes tasks. Never reasons independently. Never stores persistent state.
- Agents are executors. The router delegates tasks. Each agent has a manifest declaring its capabilities and permissions. Agents never call each other directly — all routing goes through the router agent.

### OS Layer — How Sena sees the machine

- **Subsystems:** platform/windows, platform/macos (future), platform/linux (future)
- **Rule:** Observation and execution only. Sanitizes all OS data before forwarding. Never touches inference or memory directly.
- The platform layer translates OS events into sanitized gRPC messages. It enforces agent capability containment at the OS level. Raw OS data never reaches cognitive subsystems.

### Interface Layer — What humans see

- **Subsystems:** ui/
- **Rule:** Renders state. Never owns state. No business logic.
- The UI receives all state from daemon-bus via gRPC streaming. It sends user actions to reactive-loop. It never caches state independently or makes decisions about what to show — it renders what it receives.

## 4. Subsystem Contract — Required Before Any Subsystem Exists

A subsystem DOES NOT EXIST until ALL of the following are present:

1. Directory created under the correct layer folder
2. `manifest.toml` — declares name, language, responsibility, ports, signals
3. `.github/instructions/<name>.instructions.md` — ownership rules and traps
4. `.github/agents/memory/<name>/` directory with `status.md`, `gaps.md`, `decisions.md`
5. Proto entry in `shared/proto/` if the subsystem exposes a gRPC service
6. Entry in `.github/agents/memory/_global/subsystems.md`
7. ADR in `docs/decisions/` if it deviates from existing architecture
8. Boot signal registered in `daemon-bus/config/daemon-bus.toml`

**A subsystem without all 8 is incomplete and must not be referenced by other subsystems until complete.**

Note: Existing subsystems implemented before this governance system was established are grandfathered in. Their admission gaps are tracked in their respective `gaps.md` files and will be resolved incrementally.

## 5. Data Flow Law

````
OS Events      → platform/        → daemon-bus (sanitized gRPC)
User Input     → ui/              → reactive-loop (gRPC)
Thoughts       → ctp/             → daemon-bus (event bus)
Memory reads   → any subsystem    → memory-engine (gRPC only)
Model calls    → any subsystem    → inference (gRPC only)
Prompt build   → reactive-loop    → prompt-composer (gRPC only)
Identity read  → any subsystem    → soulbox (gRPC only)
Agent tasks    → reactive-loop    → router agent → specific agent
````

**Violations of this flow are architectural defects, not shortcuts.**

## 6. Per-Subsystem File Structure

Every Rust subsystem follows this structure:

````
<subsystem>/
├── config/
│   ├── <name>.toml              # Active config (gitignored if contains secrets)
│   └── <name>.toml.example      # Committed template
├── src/
│   ├── generated/               # Proto-generated code (never hand-edited)
│   │   └── sena.daemonbus.v1.rs
│   ├── config.rs                # Config loading only
│   ├── error.rs                 # SenaError types
│   ├── main.rs                  # Boot sequence only
│   └── [domain modules]         # All business logic
├── build.rs                     # Proto codegen
├── Cargo.toml
└── manifest.toml                # Subsystem declaration
````

Python subsystem (lora-manager):

````
lora-manager/
├── config/
│   └── lora-manager.toml
├── src/
│   └── [modules]
└── pyproject.toml
````

C# subsystem (platform/windows):

````
platform/windows/
├── config/
│   └── platform-windows.toml
└── src/
    └── [modules]
````

## 7. Proto Ownership

`shared/proto/` is the source of truth for all gRPC proto definitions. All subsystem `build.rs` files reference this location.

**Rules:**
- Never define a proto message or service outside the canonical proto directory
- Never edit generated code in `src/generated/` — regenerate only
- Proto changes require updating ALL affected subsystems in the same PR
- Breaking changes require a version bump and an ADR in `docs/decisions/`
- Current proto package: `sena.daemonbus.v1`

## 8. Naming Conventions

| Element | Convention | Example |
|---|---|---|
| Subsystem directories | kebab-case | `memory-engine`, `reactive-loop` |
| Binary names | Same as subsystem | `memory-engine.exe` |
| gRPC services | PascalCase | `MemoryService`, `InferenceService` |
| Events/signals | SCREAMING_SNAKE_CASE | `MEMORY_ENGINE_READY`, `SENA_READY` |
| Config keys | snake_case | `max_entries`, `listen_port` |
| Rust modules | snake_case | `mod memory_engine` |
| TypeScript components | PascalCase | `SubsystemHealth.tsx` |
| Proto package | dot-separated lowercase | `sena.daemonbus.v1` |

## 9. Configuration Law

- Every tunable value lives in `config/<name>.toml`
- No hardcoded values in source code — thresholds, ports, timeouts, weights all come from config
- Every config file has a `.example` twin committed to git
- Sensitive values use environment variables, never config files
- Config changes that affect subsystem behavior require documentation in that subsystem's `memory/decisions.md`
