What
This change bootstraps Sena with a minimal skeleton: `daemon-bus` gRPC server, a single `agents/example-agent` that registers and communicates via gRPC, the `daemon-bus/proto/` contract for startup/health, and CI that builds and runs basic integration tests.

Why
Provide a runnable baseline to validate subsystem boundaries, gRPC contract flow, observability plumbing, and repo CI conventions so later feature work can iterate safely.

Subsystems affected
1. `daemon-bus` — proto, gRPC server, health checks
2. `agents` — example agent that talks to daemon-bus via gRPC
3. `platform/windows` — desktop integration stubs (minimal)
4. `ui` — placeholder that demonstrates UI<->daemon-bus separation (render-only stub)
5. CI / repo tooling — build, lint, tests

Assumptions
- `docs/PRD.md` has been read and no blocking architectural constraints are present.
- Team has Rust, Python, and .NET SDK installed or will follow provided environment scripts.
- No model runtime is required for the skeleton; model integration is out of scope for this bootstrapping change.

Out of scope
- Full CTP, prompt-composer, memory-engine implementations.
- Production-ready security/encryption for SoulBox (only stubs and interfaces).
- Any persistent storage migrations beyond simple in-memory or file-backed stubs.

Acceptance criteria
- A developer can `cargo build` (Rust subsystems) and run daemon-bus to accept a gRPC health call.
- An `example-agent` can connect and register with daemon-bus via the proto-defined API.
- CI pipeline builds all subsystems and runs integration test that asserts the agent registration flow.
- Tracing headers are propagated across gRPC calls in the integration test.

Risks & Mitigations
- Risk: unclear protocol details in PRD — Mitigation: early RFC/proto review with stakeholders; iterate proto in its own small PR.
- Risk: environment setup friction — Mitigation: add simple bootstrap script and documented prerequisites.

Milestones (week 0–2)
- Day 0–1: Read `docs/PRD.md`, create `PLAN.md`, align team on scope.
- Day 2–4: Implement `daemon-bus/proto/` with health and agent registration messages; wire a simple gRPC server (Rust).
- Day 5–7: Implement `agents/example-agent` (Python or Rust) that registers with daemon-bus.
- Day 8–10: Add tracing and logging (OpenTelemetry metadata propagation, `tracing` in Rust, `structlog` in Python).
- Day 11–14: Add CI (build + run integration test), create first PR following PR hygiene.

Notes
- Keep proto definitions only in `daemon-bus/proto/` (per serialization rules).
- Do not hardcode values; config should be TOML or proto constants.