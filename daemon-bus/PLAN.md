# PLAN — daemon-bus Scaffold

## What

Scaffold the complete daemon-bus crate: Cargo.toml, proto definitions, config, and all source modules — compiling, structured, ready for implementation. Stubs where logic is unimplemented, but every module compiles and the dependency graph is correct.

## Why

daemon-bus is the root process for Sena. Every other subsystem depends on it being alive and serving gRPC. Nothing else can be built until this crate exists with its proto contracts, error types, event bus, and supervision skeleton in place.

## Subsystems Affected

- **daemon-bus** — this is the target. All files are new.
- No other subsystems are modified. Proto definitions here will be consumed by other subsystems in future PRs.

## Assumptions

1. **Rust 2021 edition, tokio multi-thread runtime** — per PRD §13.1.
2. **tonic + prost for gRPC** — per PRD §13.7. Using tonic 0.12.x and prost 0.13.x (latest stable in training data). Build-time codegen via `tonic-build`.
3. **Proto package `sena.daemonbus.v1`** — per PRD §13.10 versioning policy.
4. **Config via `toml` crate + `serde`** — daemon-bus.toml is the single config source. Loaded once at startup, passed by reference. No hot-reload in V1.
5. **tracing + tracing-subscriber for logging** — per PRD §13.8. OpenTelemetry integration deferred to a follow-up PR; subscriber is set up with a JSON structured layer.
6. **No `mod.rs` files** — per copilot-instructions Rust style rules. Named module files only. The task spec says `src/boot/mod.rs` etc. but the global rule "No mod.rs — use named modules" overrides. I will use `src/boot.rs`, `src/bus.rs`, etc. as flat named modules. **Decision reversed**: The task spec is the direct instruction and explicitly requests `src/boot/mod.rs` etc., implying these are directories with sub-modules. I'll use the `mod.rs` convention as specified in the task since the intent is clearly to have expandable module directories. This is documented here as a known deviation from the global style rule, made deliberately because the task owner requested this structure.
7. **Process supervision uses `tokio::process::Command`** — child processes are OS processes spawned and watched via async process handles. On Windows, termination uses `kill()` (which sends TerminateProcess). SIGTERM semantics noted in watchdog are approximated via process kill on Windows.
8. **Broadcast channel capacity from config** — the tokio broadcast channel has a fixed capacity set from config at startup.
9. **Boot sequence signals** — the full boot signal set from PRD §3.2 is defined in proto: DAEMON_BUS_READY, MEMORY_ENGINE_READY, PLATFORM_READY, AGENTS_READY, OLLAMA_READY, MODEL_PROFILE_READY, LORA_READY, LORA_SKIPPED, CTP_READY, UI_READY, SENA_READY.
10. **Generated code directory** — `src/generated/` will contain a `.gitkeep` and a note. Actual codegen happens via `build.rs` at compile time, outputting to `OUT_DIR`. The `src/generated/` directory is a placeholder for checked-in generated code in CI, but for local dev `include!` from `OUT_DIR` is standard tonic practice.
11. **Priority tiers 0–4** — per PRD §9.2. Tier 2 exclusivity enforced with a queue.
12. **Retry policy is exactly: immediate → 5s → 30s → degraded** — 3 attempts max, per PRD §10.2 and daemon-bus.instructions.md.

## Out of Scope

- Actual gRPC service method implementations (stubs only, returning `Unimplemented`)
- OpenTelemetry collector integration (tracing-opentelemetry wiring)
- Real child process binaries (supervisor stubs spawn placeholder commands)
- Integration tests against other subsystems
- CI/CD pipeline or buf breaking checks
- Hot-reload of config
- Windows Job Object resource quota enforcement (noted in watchdog, deferred to platform layer integration)