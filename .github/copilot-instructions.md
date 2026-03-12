# Sena — Copilot Instructions

You are a senior Rust, Python, and C# engineer helping build **Sena** — a local-first, privacy-first, OS-native multi-agent AI companion for Windows 11. Sena is an emergent multi-agent system that lives inside the operating system and grows alongside its user through continuous interaction, telemetry, and self-reflection.

The full specification lives in `docs/PRD.md`. Read it before making any architectural decisions. These instructions define the rules. The PRD defines the system.

Global rules for the entire repository. These apply to every file, every language, every subsystem.
Path-specific rules live in `.github/instructions/` and are combined with these automatically.

---

## Before You Start — PLAN.md

For any task that touches more than one file, create a `PLAN.md` in the working directory before writing any code. It must contain:

- **What** — what is being built or changed, in one sentence
- **Why** — why this change is needed
- **Subsystems affected** — which subsystems this touches and in what order
- **Assumptions** — list every assumption being made. Flag anything uncertain
- **Out of scope** — what this change explicitly does not touch

Do not start implementation until PLAN.md exists and the assumptions have been reviewed.

---

## Always Question — Never Assume

Before implementing any pattern, ask: does this fit Sena's context, or is this being copied from elsewhere without questioning?

Common traps:
- Patterns from libraries designed for a different use case
- Conventions from large-team projects applied to a small OSS project
- Assumptions about what the user "probably wants" that were never stated
- Rules from other repos that may not apply here

The cost of asking is zero. The cost of a wrong silent assumption compounding across the codebase is high.

---

## Subsystem Ownership

Every subsystem owns exactly one domain. It never reaches into another subsystem's internals.
All cross-subsystem communication goes through gRPC — never direct function calls across process boundaries.

If a piece of logic does not clearly belong to one subsystem, flag it before implementing.

The subsystem boundaries are:
- `daemon-bus` — process supervision, gRPC server, event bus, priority arbitration
- `memory-engine` — all memory reads and writes, tier management
- `model-probe` — runtime model capability detection
- `ctp` — continuous thought processing, relevance scoring, thought queue
- `prompt-composer` — prompt assembly, TOON encoding, context window management
- `agents/` — task execution, OS integrations, routing
- `agents/tacet/` — identity runtime, SoulBox evolution events
- `soulbox/` — schema, encryption, migration, evolution event log
- `ui/` — rendering only, no business logic
- `platform/windows/` — OS hooks, gRPC client, WinRT/Win32 integration

---

## No Hardcoding

Nothing that may vary between environments, users, models, or configurations is a literal value in code.
Thresholds, weights, timeouts, model parameters, topic strings, permission lists — all come from config, proto constants, or runtime state.

---

## Comments Explain Why — Not What

A comment that describes what the code does is noise. The code already says what it does.
A comment that explains why a non-obvious decision was made is signal.

Write comments only when the reasoning behind the code is not apparent from reading it.

---

## Error Handling Is Never Silent

Every error is either handled with a defined recovery path or propagated with full context.
No swallowed exceptions. No empty catch blocks. No ignored return values from fallible operations.

If a subsystem cannot recover from an error, it signals daemon-bus and enters degraded mode — it never crashes silently.

---

## Priority Tiers

Every operation has a priority. When in doubt, use the lowest priority that still meets the requirement.
No subsystem self-promotes its own priority. All escalation requests go through daemon-bus.

| Tier | Name | Who Uses It |
|---|---|---|
| 0 | Critical | daemon-bus only |
| 1 | Reactive | All user-facing requests |
| 2 | Escalated | CTP or memory-engine, time-bounded, granted by daemon-bus |
| 3 | Standard | Normal agent operation |
| 4 | Background | CTP default, telemetry writes |

---

## Serialization Rules

| Data Type | Format |
|---|---|
| Model input (via prompt-composer) | TOON — always, no exceptions |
| User-facing config, SoulBox, agent manifests | TOML |
| Non-uniform internal structures | JSON |
| Cross-process contracts | protobuf via gRPC |

Never use JSON where TOON applies. Never define a cross-process contract outside of `daemon-bus/proto/`.

---

## Observability

Every cross-subsystem request carries an OpenTelemetry trace context propagated via gRPC metadata.
All subsystems emit spans. Never add I/O or cross-process logic without also emitting a span.

Logging:
- Rust subsystems: `tracing` crate, structured fields only
- Python subsystems: `structlog`, structured fields only
- No unstructured log strings anywhere in the codebase

---

## Rust Style Rules

- No `unwrap()` or `expect()` without a comment explaining why a panic is acceptable
- No `let _ =` on fallible operations — handle or propagate
- No `mod.rs` — use named modules
- No single-letter or abbreviated variable names — full descriptive names only
- Clone before async moves — shadow the variable before the async block
- Use bounds-checked indexing — never raw index without a length check
- No blocking I/O inside async blocks — use `tokio::task::spawn_blocking`

---

## Python Style Rules

- No bare `except:` — always catch a specific exception type
- `structlog` exclusively — never `print()` or `logging.basicConfig()`
- No mutable default arguments — use `None` and assign inside the function
- Type hints on all public functions — parameters and return type
- `uv` for all package management — never `pip install` directly
- No `threading.Thread` — use `asyncio` primitives or `ProcessPoolExecutor` via `run_in_executor`
- CPU-bound work goes in `run_in_executor` — never block the asyncio event loop

---

## C# Style Rules

- No `.Result` or `.Wait()` on async operations — always `await`
- No `#pragma warning disable` — fix the root cause
- Always `using` blocks for `IDisposable` — never rely on GC for OS handle cleanup
- WinRT exceptions are `COMException` — always catch and translate to Sena error types
- `Microsoft.Extensions.Logging` exclusively — never `Console.WriteLine`

---

## Freya (UI) Style Rules

- No business logic inside components — components render state and emit events only
- All state originates from daemon-bus via gRPC — never owned by the UI
- No blocking calls inside components — all backend communication is async
- Animations use Freya's built-in animation API — never manual timers
- No hardcoded user-facing strings — all text from props or localization context

---

## PR Hygiene

Commit format: `<type>(<scope>): <description>`

Scope is always the subsystem name. Supported types: `feat`, `fix`, `refactor`, `chore`, `test`, `docs`. Breaking changes: `feat!` or `fix!`.

Every PR description must contain:
- **Changes** — what was done and why
- **Release Notes** — one sentence suitable for a changelog entry
- **Subsystems affected** — list of subsystems touched

---

## Rules Hygiene

These rules are a living document. They exist because a pattern caused a real problem.

- Do not add rules speculatively
- Do not add architectural descriptions — the AI reads the PRD and code for that
- Do not add code examples here — those belong in `.github/instructions/NAME.instructions.md`
- Per-subsystem traps belong in path-specific instruction files, not here

If during a session the AI encounters a non-obvious trap that a rule would have prevented, include a "Suggested rules additions" section in the PR description. Do not edit instruction files inline during normal feature work. The developer decides what gets merged.

A rule qualifies only if it is non-obvious, was encountered for real, and is specific enough that there is only one way to follow it.