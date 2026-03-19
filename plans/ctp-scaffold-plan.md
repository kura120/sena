# Plan: CTP Subsystem Scaffold

**Created:** 2026-03-18
**Status:** Ready for Atlas Execution

## Summary

Scaffold the `ctp` Rust subsystem — Sena's continuous proactive cognitive loop. CTP runs permanently at background priority, generating candidate thoughts from telemetry and memory signals, scoring them for relevance, surfacing high-relevance ones through the thought queue, and driving memory consolidation during idle periods. Three parallel async pipelines (generation, evaluation, consolidation) run independently with no shared mutable state. This plan takes CTP from zero to `cargo check` clean with the full pipeline structure in place and `TOPIC_THOUGHT_SURFACED` firing on daemon-bus. Completing this subsystem closes **Milestone A — Sena is alive**.

## Context & Analysis

**Relevant Files:**
- `daemon-bus/proto/sena.daemonbus.v1.proto`: `TOPIC_THOUGHT_SURFACED`, `TOPIC_SESSION_COMPACTION_TRIGGERED`, `TOPIC_MEMORY_CONSOLIDATION_REQUESTED`, `CTP_READY` boot signal — confirm these are present after the proto edit
- `inference/src/main.rs`: Reference for daemon-bus connect + boot signal gate + tracing init pattern — copy exactly
- `inference/src/error.rs`: Reference for `thiserror` + `From<E> for tonic::Status` pattern
- `inference/src/config.rs`: Reference for `Config::load` + validation pattern
- `memory-engine/src/grpc.rs`: Reference for daemon-bus gRPC publish pattern
- `.github/copilot-instructions.md`: Global rules — all apply in full
- `.github/instructions/ctp.instructions.md`: Subsystem-specific rules — all apply in full

**Key Architectural Constraints from Instructions:**
- CTP never sleeps — no `tokio::time::sleep` in the core loop. Loops yield by awaiting stream input or queue pops
- Three parallel pipelines: generation, evaluation, consolidation — separate `tokio::spawn` tasks, never collapsed
- Thought queue is priority-ordered by relevance score, never FIFO
- Every thought has an expiry — derived from score via config, never hardcoded
- Relevance weights come from `SoulBoxSnapshot` — never constants. For Phase 1 (SoulBox not yet built), use a `DefaultWeights` fallback struct from config
- Surface threshold is dynamic per `ActivityState` — `UserActive=0.9`, `Idle2Min=0.6`, `Idle10Min=0.3` — thresholds from config
- Activity detection via Win32 `GetLastInputInfo` — polled on a dedicated background task, cached as `Arc<AtomicU8>`
- Discarded thoughts logged at `trace` only — never `debug` or higher
- Escalation always goes through daemon-bus gRPC — never self-promote
- Memory consolidation only during deep idle — CTP sends gRPC request to memory-engine, never writes directly
- `TOPIC_THOUGHT_SURFACED` published to daemon-bus when a thought crosses the surface threshold

**Dependencies:**
- `tokio` full features
- `tonic` + `prost`: daemon-bus gRPC client
- `tonic-build`: build dep
- `thiserror`: error type
- `serde` + `toml`: config
- `tracing` + `tracing-subscriber`: structured logging
- `windows` crate with `Win32_UI_Input_KeyboardAndMouse` + `Win32_System_SystemInformation` features: `GetLastInputInfo`, `GetTickCount`
- `uuid` with v4 feature: thought IDs
- `ordered-float`: `f32` ordering for `BinaryHeap`
- `tokio-stream`: stream utilities
- `chrono`: timestamps

**Patterns & Conventions:**
- No `unwrap()`/`expect()` without comment
- No `let _ =` on fallible ops
- No `tokio::time::sleep` in the thought generation loop
- Write lock never held across `await`
- Named modules only — no `mod.rs`
- All thresholds from `config/ctp.toml`
- Commit format: `feat(ctp): <description>`

## Implementation Phases

---

### Phase 1: Foundation — Cargo.toml, manifest, build.rs, proto regeneration

**Objective:** Crate defined, proto bindings regenerated, `cargo check` clean with empty `main.rs`.

**Files to Modify/Create:**
- `ctp/Cargo.toml`
- `ctp/manifest.toml`
- `ctp/build.rs` — copy tonic-build pattern from `inference/build.rs` exactly
- `ctp/src/main.rs` — `fn main() {}` placeholder
- Workspace `Cargo.toml` — add `ctp` to `[workspace.members]` if absent

**Tests to Write:**
- None — compilation is the acceptance criterion

**Steps:**
1. Read `inference/build.rs` and workspace `Cargo.toml` for dep patterns
2. Create `ctp/Cargo.toml` with all required deps — check workspace-level deps before adding new ones
3. Create `ctp/manifest.toml`:
   ```toml
   name = "ctp"
   language = "rust"
   responsibility = "Continuous proactive cognitive loop — thought generation, relevance evaluation, memory consolidation"
   capability_flags = ["CTP_READY"]
   ```
4. Create `ctp/build.rs` pointing at `../daemon-bus/proto/sena.daemonbus.v1.proto`, output to `src/generated/`
5. Create `ctp/src/main.rs` with `fn main() {}`
6. Add `ctp` to workspace members if missing
7. `cargo check -p ctp` — zero errors, zero warnings

**Acceptance Criteria:**
- [ ] `cargo check -p ctp` passes clean
- [ ] Proto-generated types visible from the crate
- [ ] `manifest.toml` present with all four required fields

---

### Phase 2: error.rs + config.rs

**Objective:** Error type and config struct. CTP's config is richer than inference's — it carries all relevance weights, expiry windows, surface thresholds, and consolidation policy as defaults (overridden by SoulBox at runtime when SoulBox is available).

**Files to Modify/Create:**
- `ctp/src/error.rs`
- `ctp/src/config.rs`
- `ctp/config/ctp.toml`
- `ctp/src/main.rs` — add `mod error; mod config;`

**Tests to Write:**
- `test_config_load_valid`: load fixture, assert all fields present
- `test_config_validates_surface_thresholds_in_range`: assert thresholds between 0.0 and 1.0, error otherwise
- `test_config_validates_expiry_windows_nonzero`: assert expiry window durations > 0
- `test_error_display_messages`: assert each `CtpError` variant has a non-empty display string
- `test_default_weights_sum_to_reasonable_value`: assert default relevance weights are > 0.0 and produce a score in [0.0, 1.0] for a mid-range input

**`ctp.toml` required keys:**
- `[surface_thresholds]` — `user_active`, `idle_2min`, `idle_10min` (f32 each)
- `[expiry_windows]` — `high_relevance_secs`, `medium_relevance_secs`, `low_relevance_secs` (u64 each)
- `[default_weights]` — `urgency`, `emotional_resonance`, `novelty`, `recurrence`, `idle_curiosity` (f32 each)
- `[consolidation]` — `idle_threshold_secs` (u64), `promotion_min_score` (f32), `max_entries_per_cycle` (u32)
- `[compaction]` — `pre_rot_fraction` (f32, default 0.8), `max_entries_to_summarize` (u32)
- `[queue]` — `max_depth` (u32)
- `[activity]` — `poll_interval_ms` (u64), `idle_2min_threshold_secs` (u64), `idle_10min_threshold_secs` (u64)

**Steps:**
1. Write tests (fail)
2. Define `CtpError` with variants: `Config(String)`, `DaemonBus(#[from] tonic::transport::Error)`, `MemoryEngine(String)`, `QueueFull`, `ActivityDetection(String)`
3. Define `Config` struct with nested `SurfaceThresholds`, `ExpiryWindows`, `DefaultWeights`, `ConsolidationConfig`, `CompactionConfig`, `QueueConfig`, `ActivityConfig`
4. Implement `Config::load` with validation
5. Write `ctp/config/ctp.toml` with all keys and sensible defaults
6. Run tests — pass
7. `cargo check -p ctp` — clean

**Acceptance Criteria:**
- [ ] All 5 tests pass
- [ ] `ctp.toml` has all required sections and keys
- [ ] Surface thresholds validated in (0.0, 1.0]
- [ ] Expiry windows validated > 0
- [ ] `cargo check -p ctp` zero warnings

---

### Phase 3: activity.rs — Win32 Idle Detection

**Objective:** Background activity state detector. Polls `GetLastInputInfo` on a dedicated task, caches result as `Arc<AtomicU8>`, exposes `ActivityState` enum. The hot path reads the atomic — never calls Win32 directly.

**Files to Modify/Create:**
- `ctp/src/activity.rs`
- `ctp/src/main.rs` — add `mod activity;`

**Tests to Write:**
- `test_activity_state_from_u8_roundtrip`: assert `ActivityState::from(state.as_u8()) == state` for all variants
- `test_surface_threshold_per_state`: assert `ActivityState::UserActive.surface_threshold(&config) == config.surface_thresholds.user_active`, same for other states
- `test_is_deep_idle_only_for_idle_10min`: assert only `Idle10Min` returns `true` for `is_deep_idle()`
- `test_atomic_cache_updates`: create an `ActivityMonitor`, manually set the atomic to `Idle2Min`, assert `current_state()` returns `Idle2Min`

**Steps:**
1. Write tests (fail)
2. Define `ActivityState` enum: `UserActive`, `Idle2Min`, `Idle10Min` with `as_u8()`, `From<u8>`, `surface_threshold(&Config) -> f32`, `is_deep_idle() -> bool`
3. Define `ActivityMonitor` with `Arc<AtomicU8>` inner cache
4. Implement `fn idle_duration_ms() -> u64` using `GetLastInputInfo` + `GetTickCount` from the `windows` crate — inside `unsafe` block, documented
5. Implement `async fn run_poll_loop(&self, config: &ActivityConfig)` — `loop { tokio::time::sleep(poll_interval); let duration = spawn_blocking(idle_duration_ms); update atomic; }` — this is the ONE place `sleep` is acceptable, it's the activity monitor not the thought loop
6. Implement `fn current_state(&self) -> ActivityState` — reads atomic, maps to enum
7. Run tests — pass
8. `cargo check -p ctp` — clean

**Acceptance Criteria:**
- [ ] All 4 tests pass
- [ ] Win32 calls inside `unsafe` with documented safety reasoning
- [ ] Hot path `current_state()` reads only the atomic — zero Win32 calls on the hot path
- [ ] Poll loop uses `spawn_blocking` for the `GetLastInputInfo` call
- [ ] `cargo check -p ctp` zero warnings

---

### Phase 4: thought_queue.rs

**Objective:** Priority-ordered async thought queue. Thoughts are ordered by relevance score (highest first). Expired thoughts are silently discarded at pop time. Max depth enforced at push time.

**Files to Modify/Create:**
- `ctp/src/thought_queue.rs`
- `ctp/src/main.rs` — add `mod thought_queue;`

**Tests to Write:**
- `test_push_and_pop_returns_thought`: push one, pop one, assert content matches
- `test_priority_ordering_high_before_low`: push score=0.3 then score=0.9, assert 0.9 pops first
- `test_expired_thought_discarded_silently`: push thought with `expires_at` in past, pop returns `None`
- `test_queue_full_returns_error`: fill to `max_depth`, assert next push returns `QueueFull`
- `test_expiry_window_from_score`: assert high score (>0.8) gets `high_relevance_secs` window, medium gets medium, low gets low
- `test_pop_wakes_on_push`: spawn pop task, push after 10ms, assert pop receives thought

**Steps:**
1. Write tests (fail)
2. Define `Thought` struct: `id: Uuid`, `content: String`, `score: f32`, `expires_at: Instant`, `generated_at: Instant`
3. Implement `Ord`/`PartialOrd` for `Thought` by `score` descending (highest score = highest priority in `BinaryHeap`)
4. Define `ThoughtQueue` with `Mutex<BinaryHeap<Thought>>` + `Notify`
5. Implement `push(thought: Thought, max_depth: usize) -> Result<(), CtpError>`: check depth, push, notify
6. Implement `async fn pop(&self) -> Option<Thought>`: await notify, pop from heap, check `expires_at < Instant::now()` → discard and try again, return `Some(thought)` or `None`
7. Implement `fn expiry_for_score(score: f32, config: &ExpiryWindows) -> Instant` — bucket by score range into the three expiry window durations
8. Run tests — pass
9. `cargo check -p ctp` — clean

**Acceptance Criteria:**
- [ ] All 6 tests pass
- [ ] `BinaryHeap` pops highest score first
- [ ] Expired thoughts discarded silently at `pop` — never emitted
- [ ] `push` returns `CtpError::QueueFull` at max depth
- [ ] Expiry window is score-derived from config — never hardcoded
- [ ] `cargo check -p ctp` zero warnings

---

### Phase 5: relevance.rs — Scoring Engine

**Objective:** Compute a relevance score in [0.0, 1.0] for a candidate thought given a set of input signals and relevance weights. Weights come from a `WeightsSnapshot` — either from SoulBox (Phase 3) or the config default fallback (Phase 1).

**Files to Modify/Create:**
- `ctp/src/relevance.rs`
- `ctp/src/main.rs` — add `mod relevance;`

**Tests to Write:**
- `test_score_clamps_to_unit_interval`: assert score is always in [0.0, 1.0] for any input combination
- `test_zero_weights_produces_zero_score`: all weights 0.0 → score 0.0
- `test_urgency_dominates_when_high`: high urgency signal with max weight → score approaches 1.0
- `test_score_increases_with_signal_strength`: same weights, stronger signal → higher score
- `test_default_weights_produce_mid_range_score`: mid-level signals with default weights → score in (0.2, 0.8)
- `test_score_is_deterministic`: same inputs always produce same score

**Steps:**
1. Write tests (fail)
2. Define `SignalInput` struct: `urgency: f32`, `emotional_resonance: f32`, `novelty: f32`, `recurrence: f32`, `idle_curiosity: f32` — all in [0.0, 1.0]
3. Define `WeightsSnapshot` struct mirroring `DefaultWeights` config — loaded from config or SoulBox snapshot
4. Implement `fn compute_score(signals: &SignalInput, weights: &WeightsSnapshot) -> f32`:
   - Weighted sum: `(urgency * w.urgency + emotional_resonance * w.emotional_resonance + ...) / total_weight`
   - Clamp result to [0.0, 1.0]
   - Pure function — no side effects, no I/O
5. Implement `fn weights_from_config(config: &DefaultWeights) -> WeightsSnapshot` — the fallback until SoulBox is live
6. Run tests — pass
7. `cargo check -p ctp` — clean

**Acceptance Criteria:**
- [ ] All 6 tests pass
- [ ] `compute_score` is a pure function — no side effects
- [ ] Output always clamped to [0.0, 1.0]
- [ ] `WeightsSnapshot` can be constructed from config (default) or SoulBox snapshot (future)
- [ ] `cargo check -p ctp` zero warnings

---

### Phase 6: context_assembler.rs

**Objective:** Assemble a `PromptContext` struct from memory-engine gRPC responses, activity state, and SoulBox snapshot (stubbed for Phase 1). This is the handoff to prompt-composer — CTP assembles, PC serializes. Never TOON-encodes inside CTP.

**Files to Modify/Create:**
- `ctp/src/context_assembler.rs`
- `ctp/src/main.rs` — add `mod context_assembler;`

**Tests to Write:**
- `test_assembler_produces_context_with_all_fields`: given mock memory responses and activity state, assert all `PromptContext` fields are populated
- `test_assembler_does_not_toon_encode`: assert output `PromptContext` contains raw Rust structs — no TOON strings
- `test_assembler_parallel_memory_queries`: assert short-term, long-term, and episodic memory reads are fired concurrently via `tokio::join!` — not sequentially
- `test_assembler_uses_empty_soulbox_when_unavailable`: when SoulBox gRPC returns unavailable, assert assembler falls back to `SoulBoxSnapshot::empty()` without error

**Steps:**
1. Write tests (fail)
2. Define `PromptContext` struct:
   ```rust
   pub struct PromptContext {
       pub soulbox_snapshot: SoulBoxSnapshot,
       pub short_term: Vec<MemoryEntry>,
       pub long_term: Vec<MemoryEntry>,
       pub episodic: Vec<MemoryEntry>,
       pub os_context: OsContext,
       pub model_profile: ModelCapabilityProfile,
       pub user_intent: Option<String>,
       pub activity_state: ActivityState,
   }
   ```
3. Define stub types `SoulBoxSnapshot::empty()`, `OsContext::empty()` — real implementations come in later milestones
4. Define `ContextAssembler` with memory-engine gRPC client and model profile cache
5. Implement `async fn assemble(&self, activity: ActivityState) -> Result<PromptContext, CtpError>`:
   - Fire short-term, long-term, episodic memory reads via `tokio::join!` (parallel, never sequential)
   - Attempt SoulBox read — fallback to `SoulBoxSnapshot::empty()` on error (SoulBox not built yet)
   - Populate `OsContext::empty()` (platform layer not built yet)
   - Build and return `PromptContext`
6. Run tests — pass
7. `cargo check -p ctp` — clean

**Acceptance Criteria:**
- [ ] All 4 tests pass
- [ ] Three memory tier reads use `tokio::join!` — provably parallel
- [ ] `PromptContext` contains no TOON-encoded strings — raw types only
- [ ] SoulBox unavailability produces fallback, not error
- [ ] `cargo check -p ctp` zero warnings

---

### Phase 7: pipelines.rs — Three Parallel Async Pipelines

**Objective:** The three core CTP pipelines as separate async tasks. Each owns its own context slice. They share no mutable state. Generation pushes to `ThoughtQueue`. Evaluation reads from queue and surfaces above-threshold thoughts. Consolidation runs during deep idle only.

**Files to Modify/Create:**
- `ctp/src/pipelines.rs`
- `ctp/src/main.rs` — add `mod pipelines;`

**Tests to Write:**
- `test_generation_pipeline_pushes_to_queue`: feed a synthetic telemetry signal, assert a thought appears in the queue
- `test_evaluation_pipeline_surfaces_high_score`: push a thought with score above `user_active` threshold, assert `TOPIC_THOUGHT_SURFACED` is published to daemon-bus
- `test_evaluation_pipeline_discards_low_score`: push thought below threshold, assert it is not surfaced (logged at trace only)
- `test_consolidation_pipeline_skips_when_active`: assert consolidation gRPC call is NOT made when `ActivityState::UserActive`
- `test_consolidation_pipeline_fires_when_deep_idle`: assert `TOPIC_MEMORY_CONSOLIDATION_REQUESTED` is published when `ActivityState::Idle10Min`
- `test_pipelines_do_not_share_mutable_state`: structural — three pipelines accept only immutable refs or `Arc` to shared state — no `Mutex<SharedContext>` spanning all three

**Steps:**
1. Write tests (fail)
2. Define `GenerationPipeline`: reads telemetry stream (stubbed as a channel for Phase 1), computes `SignalInput`, calls `compute_score`, builds `Thought`, pushes to `ThoughtQueue`. Never blocks. Uses `tokio::spawn` internally for thought construction
3. Define `EvaluationPipeline`: pops from `ThoughtQueue` in a loop, compares score to `activity_monitor.current_state().surface_threshold(&config)`, if above → publish `TOPIC_THOUGHT_SURFACED` to daemon-bus, if below → `tracing::trace!` discard
4. Define `ConsolidationPipeline`: loop with a `tokio::time::sleep(idle_check_interval)` (this is the one acceptable sleep in CTP — it's not the thought loop), check `activity_monitor.is_deep_idle()`, if true → publish `TOPIC_MEMORY_CONSOLIDATION_REQUESTED` to daemon-bus
5. Implement `fn spawn_all(config, queue, activity_monitor, daemon_bus_client, memory_client) -> (JoinHandle, JoinHandle, JoinHandle)` — spawns all three as independent tasks
6. Run tests — pass
7. `cargo check -p ctp` — clean

**Acceptance Criteria:**
- [ ] All 6 tests pass
- [ ] Three pipelines are three separate `tokio::spawn` tasks — never collapsed into one
- [ ] Low-score discards logged at `trace` only — no `debug` or higher
- [ ] Consolidation only fires during `Idle10Min`
- [ ] `TOPIC_THOUGHT_SURFACED` published to daemon-bus for high-score thoughts
- [ ] `TOPIC_MEMORY_CONSOLIDATION_REQUESTED` published to daemon-bus during deep idle
- [ ] `cargo check -p ctp` zero warnings

---

### Phase 8: main.rs — Boot Sequence, Signal Emission, Shutdown

**Objective:** Wire everything. Boot: config → tracing → daemon-bus connect → wait `MEMORY_ENGINE_READY` + `MODEL_PROFILE_READY` + `LORA_READY|LORA_SKIPPED` → init all components → spawn three pipelines → emit `CTP_READY`. Graceful shutdown on SIGTERM/Ctrl-C.

**Files to Modify/Create:**
- `ctp/src/main.rs` — full implementation

**Tests to Write:**
- `test_boot_gate_requires_all_three_signals`: assert CTP does not emit `CTP_READY` until all three prerequisite signals are received
- `test_shutdown_cancels_all_pipeline_handles`: trigger shutdown, assert all three `JoinHandle`s are aborted cleanly

**Steps:**
1. Read `inference/src/main.rs` for the exact boot gate pattern — copy it, adapt for three signals instead of one
2. Write tests (fail)
3. Implement `#[tokio::main] async fn main()`:
   - `Config::load("config/ctp.toml")`
   - Init `tracing_subscriber` JSON
   - Connect to daemon-bus gRPC
   - Connect to memory-engine gRPC client
   - Boot gate: subscribe to boot signals, wait for `MEMORY_ENGINE_READY` AND `MODEL_PROFILE_READY` AND (`LORA_READY` OR `LORA_SKIPPED`) — all three required before proceeding
   - Initialize `ActivityMonitor`, spawn `run_poll_loop` task
   - Initialize `ThoughtQueue`
   - Initialize `ContextAssembler` with memory client
   - Spawn three pipelines via `spawn_all` — store three `JoinHandle`s
   - Emit `CTP_READY` to daemon-bus
   - Await `ctrl_c()` or SIGTERM
   - On shutdown: abort all three handles, emit nothing (CTP going down is not an error signal)
4. Run tests — pass
5. `cargo check -p ctp` — zero errors, zero warnings
6. Final sweep: grep for `unwrap`, `expect`, `let _ =`, `todo!()`, `sleep` in non-activity files — each must be justified or removed

**Acceptance Criteria:**
- [ ] Boot gate tests pass
- [ ] Shutdown test passes
- [ ] `CTP_READY` only emitted after all three prerequisite signals received
- [ ] All three pipeline `JoinHandle`s stored and aborted on shutdown
- [ ] `cargo check -p ctp` zero errors, zero warnings
- [ ] Zero unjustified `unwrap`/`expect`/`todo!()`/`let _ =` across all files
- [ ] Zero `tokio::time::sleep` outside `activity.rs` and `consolidation_pipeline`

---

## Open Questions

1. **Telemetry stream source for generation pipeline** — the platform layer (OS hooks) is not built yet. Phase 7 stubs generation with a synthetic `mpsc` channel.
   - **Option A:** Generation pipeline receives `impl Stream<Item = TelemetryEvent>` — in Phase 1, caller passes a channel receiver that emits nothing; real telemetry wired later
   - **Option B:** Generation pipeline polls memory-engine for recent entries as a proxy for telemetry signals
   - **Recommendation:** Option A. The interface is clean and the stub is trivially replaceable. No fake signals — just an empty stream for now.

2. **SoulBox weights fallback** — SoulBox is not built until Milestone C. Relevance weights must come from somewhere.
   - **Option A:** `weights_from_config()` — weights live in `ctp.toml` as defaults, SoulBox overrides them when available
   - **Option B:** Hardcode default weights in code until SoulBox exists
   - **Recommendation:** Option A. Already specified in the plan. Never hardcode weights.

## Risks & Mitigation

- **Risk:** Three pipeline JoinHandles are dropped accidentally, cancelling the tasks
  - **Mitigation:** `spawn_all` returns all three handles explicitly. `main.rs` stores them. Code-Review verifies handles are held until shutdown

- **Risk:** Consolidation pipeline fires during active use, causing unnecessary gRPC calls
  - **Mitigation:** Phase 7 test explicitly asserts consolidation does NOT fire when `UserActive`

- **Risk:** Thought queue accumulates unboundedly during high signal periods
  - **Mitigation:** `ThoughtQueue::push` enforces `config.queue.max_depth` — excess thoughts are dropped, not queued

## Success Criteria

- [ ] `cargo check -p ctp` passes clean after every phase
- [ ] All tests pass after every phase
- [ ] `TOPIC_THOUGHT_SURFACED` fires on daemon-bus when a thought exceeds the surface threshold
- [ ] `TOPIC_MEMORY_CONSOLIDATION_REQUESTED` fires during `Idle10Min` only
- [ ] `CTP_READY` emitted after all boot prerequisites satisfied
- [ ] Three pipelines are provably independent — no shared mutable state
- [ ] Zero hardcoded relevance weights, thresholds, or expiry windows
- [ ] Zero `tokio::time::sleep` outside `activity.rs` and consolidation loop

## Notes for Atlas

- **Read `inference/src/main.rs` before writing `ctp/src/main.rs`** — the boot gate pattern is already proven there, copy and adapt it for three signals
- **Phase order is strict** — each phase must `cargo check` clean before proceeding
- **Sisyphus handles all phases** — no frontend involvement
- **Telemetry stream is stubbed in Phase 7** — do not block the phase on the platform layer. Pass an `impl Stream<Item = TelemetryEvent>` that emits nothing for now
- **SoulBox is stubbed** — `SoulBoxSnapshot::empty()` is the correct fallback. Do not add a SoulBox gRPC dependency; the connection will be wired in Milestone C
- **After Phase 8 passes** — Milestone A is complete. The debug UI is the next task after this plan closes

- **Overnight run — fully automated:** I have pre-approved all phases. Run all 8 phases fully automatically — do not stop for plan approval, commit confirmation, or completion review. For each phase, create a new feature branch off dev (e.g. ctp/phase-1, ctp/phase-2), commit the work there, open a PR targeting the dev branch (NOT main), and merge it before proceeding to the next phase. Run all 8 phases to completion without pausing.

- **Activity detection must be trait-abstracted:** `activity.rs` should define an `ActivityDetector` trait with `fn idle_duration_ms() -> u64`. The Windows implementation using `GetLastInputInfo` lives in `activity_windows.rs` and is selected via `#[cfg(target_os = "windows")]`. Non-Windows builds get a stub that always returns 0 (UserActive). This keeps the Win32 code isolated and swappable for macOS/Linux in future platform milestones without touching the rest of CTP.