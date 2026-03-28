# Sena — AI and Developer Rules

These are non-negotiable. They exist because violations were observed or are predictable given the architecture. Every rule has a reason.

---

## 0. The Prime Rule

**Read before you write.**

Before touching any subsystem, read:

1. `docs/PRD.md` — What Sena is and is not
2. `docs/ARCHITECTURE.md` — Structure law and subsystem contracts
3. `docs/AI_RULES.md` — This file
4. `.github/instructions/<subsystem>.instructions.md` — Subsystem-specific rules
5. `.github/agents/memory/<subsystem>/status.md` — Current implementation state
6. `.github/agents/memory/<subsystem>/gaps.md` — Known gaps
7. `.github/agents/memory/<subsystem>/decisions.md` — Subsystem-specific decisions

If any of these files do not exist, **STOP** and create them before proceeding.

---

## 1. Technology Stack — Non-Negotiable

| Layer | Technology | Reason |
|---|---|---|
| Core subsystems | Rust | Performance, safety, no GC pauses |
| UI | Tauri v2 + React + TypeScript | Cross-platform overlay, WebView2 |
| LoRA training | Python | No Rust alternative at parity |
| Windows OS hooks | C#/.NET | WinRT/Win32 API surface |
| macOS OS hooks (future) | Swift | AppKit/CoreServices |
| Linux OS hooks (future) | Rust | D-Bus/X11/Wayland |
| IPC | gRPC (tonic) | Type-safe, cross-language, cross-process |
| Proto definitions | protobuf | Single source of truth in proto directory |
| Memory backend | ech0 (redb + usearch) | Local-first, no external process |
| Inference backend | llama-cpp-rs | Direct control, no external process |
| Rust logging | tracing crate | Structured only |
| Python logging | structlog | Structured only |
| C# logging | Microsoft.Extensions.Logging | Structured only |
| Rust async | tokio | Multi-thread scheduler |

**No substitutions without an ADR in `docs/decisions/` and explicit approval.**

---

## 2. Subsystem Rules

### 2.1 No New Subsystem Without Full Admission

A subsystem does not exist until all 8 admission requirements are met (see `ARCHITECTURE.md` §4). Writing code before admission is complete will be reverted.

### 2.2 One Domain Per Subsystem

A subsystem that does two things is two subsystems. If you find yourself adding a second domain to an existing subsystem, stop and evaluate whether a new subsystem is warranted. If so, go through admission.

### 2.3 No Cross-Subsystem Direct Calls

All cross-subsystem communication goes through gRPC. No shared memory, no direct function calls across process boundaries, no shared state.

Violation example (forbidden):
```rust
// FORBIDDEN: memory-engine calling inference directly
use inference::InferenceEngine;
```

Correct:
```rust
// CORRECT: gRPC client to inference
let response = inference_client.complete(request).await?;
```

### 2.4 OS Data Is Always Sanitized

Any data originating from the OS (file paths, process names, screen content, browser content) MUST pass through `platform/` sanitization before reaching any cognitive subsystem. OS data reaching inference or memory without sanitization is a security defect.

### 2.5 Agents Cannot Self-Elevate

Agents cannot acquire permissions beyond their declared `manifest.toml` permissions. No agent can call another agent directly — all routing goes through the router agent. Router decisions go through daemon-bus.

---

## 3. Code Quality Rules

### 3.1 No Hardcoded Values

Every tunable value lives in `config/<name>.toml`. Values that appear in more than one place create drift. If you find a hardcoded threshold, port, timeout, or string — move it to config immediately.

### 3.2 No Silent Failures

Every error either has a recovery path or propagates with full context. Empty catch blocks, swallowed errors, and ignored return values are defects. The error chain must always be traceable.

### 3.3 No `unwrap()` Without Justification

`unwrap()` and `expect()` are only acceptable when:
- The value is guaranteed by invariant (document the invariant in a comment)
- The failure is fatal infrastructure (e.g., tokio runtime construction)

Every other `unwrap()` must be replaced with proper error handling.

### 3.4 No Blocking I/O in Async Contexts

CPU-bound work in async blocks starves the executor. Use `tokio::task::spawn_blocking` for any synchronous/blocking operation inside an async function. This includes all llama-cpp-rs calls.

### 3.5 Structured Logging Only

No `println!`, `print!`, `eprintln!`, `Console.WriteLine`, `print()`.

| Language | Logger | Requirement |
|---|---|---|
| Rust | `tracing` crate | Structured fields only |
| Python | `structlog` | Structured fields only |
| C# | `Microsoft.Extensions.Logging` | Structured fields only |

Required fields on every log event vary by subsystem — see the subsystem's instruction file.

### 3.6 No Raw Hex Colors in UI

All colors come from CSS variables defined in theme files. No `#3b82f6` in component files. Ever.

### 3.7 No Hardcoded Strings in UI

All user-facing text comes from constants files. No string literals in React components.

---

## 4. Security Rules

### 4.1 All gRPC Services Bind to 127.0.0.1 by Default

Every subsystem gRPC server defaults to `listen_address = "127.0.0.1"`. Binding to `0.0.0.0` exposes services to the local network. Sena is a local-first, privacy-first application — LAN exposure is a privacy violation. Only bind to `0.0.0.0` with explicit user opt-in.

### 4.2 SoulBox Encryption Is Non-Negotiable

SoulBox data never touches disk in plaintext. AES-256-GCM with Argon2id key derivation. Fresh nonce on every write. Key never stored. Violating this is not a bug — it is a fundamental breach of user trust.

### 4.3 No User Data in Logs

Raw prompt content, model responses, SoulBox values, memory entry content, and user messages NEVER appear in logs or telemetry. Only operation names, error codes, latency, and anonymized IDs are logged.

### 4.4 No Model Output Executed as Code

Model output is text. It is never passed to `eval()`, `exec()`, `subprocess`, or any code execution path. Models can suggest code — humans or explicit tool-use agents run it with appropriate sandboxing.

### 4.5 Agent Capabilities Enforced at OS Level

Agent permissions declared in `manifest.toml` are enforced by the platform layer at spawn time, not by the agent itself. An agent claiming permissions it wasn't granted is a security defect, not a configuration error.

### 4.6 No Community Model Without Signature Verification

Externally-sourced GGUF files and LoRA weights require cryptographic signature verification before loading. Unsigned artifacts are rejected. This is not optional — malformed GGUF files can be attack vectors.

---

## 5. Architecture Rules

### 5.1 Proto First, Implementation Second

Before any new gRPC service or message is implemented, the proto definition must exist in the proto directory and be committed. Implementing a gRPC service without a proto definition first is working backwards.

### 5.2 Boot Sequence Is Sacred

The boot sequence defined in `daemon-bus/config/daemon-bus.toml` is the contract. No subsystem starts before its dependencies signal ready. No dependency is bypassed "temporarily" — bypasses become permanent.

### 5.3 Memory Writes Are Explicit

Nothing writes to memory-engine without explicit intent. CTP decides when to promote. reactive-loop writes conversation turns explicitly. No subsystem writes to memory as a side effect of doing something else.

### 5.4 All Escalations Through daemon-bus

No subsystem self-promotes priority. All Tier 2 escalation requests go through daemon-bus ArbitrationService. Tier 2 is exclusive — only one holder at a time. Expiry is enforced — no indefinite escalations.

### 5.5 TOON Is the Preferred Model Input Format

All structured data fed to the model goes through prompt-composer's ESU (Encoding Selection Utility). The ESU decides TOON vs JSON based on token efficiency — TOON is used when it achieves >15% token savings over JSON. Bypassing ESU to pass raw data directly to the model is an architectural violation.

### 5.6 Decisions Are Documented

Any deviation from this document, `PRD.md`, or `ARCHITECTURE.md` requires:

1. An ADR in `docs/decisions/`
2. An entry in `.github/agents/memory/_global/architecture.md`
3. An entry in the affected subsystem's `memory/decisions.md`

Undocumented deviations will be caught by the next audit and flagged as defects regardless of whether the deviation was beneficial.

---

## 6. What Requires an ADR

An Architectural Decision Record (ADR) in `docs/decisions/` is required for:

- Adding a new subsystem
- Changing the technology for any layer (e.g., replacing a crate)
- Changing the data flow between subsystems
- Adding a new gRPC service or major RPC
- Changing the boot sequence dependency graph
- Any security model change
- Deviating from a rule in this document

See `docs/decisions/TEMPLATE.md` for the ADR format.
