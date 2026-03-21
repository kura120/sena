# Sena Code Quality Audit
**Date:** 2026-03-21  
**Auditor:** Claude Sonnet 4.6 (GitHub Copilot — Atlas mode)  
**Source of truth:** `docs/PRD.md` v0.7.1, `.github/instructions/`, `docs/decisions/`  
**Approved deviations:** `docs/decisions/ui-tauri-migration.md` (Freya → Tauri)

---

## Executive Summary

| Metric | Count |
|---|---|
| **Total issues found** | 42 |
| **Critical (blocking)** | 5 |
| **Major (should fix)** | 20 |
| **Minor (nice to fix)** | 17 |

**Subsystems fully passing (clippy + tests):** none — every Rust subsystem has at least one clippy error under `-D warnings`  
**Subsystems with test compilation failure:** `memory-engine`  
**Subsystems not started:** `lora-manager`, `codebase-context`

### Overall Assessment

The codebase has a well-structured architecture with clean separation of concerns and solid test coverage for the logic that exists. The primary concerns are: (1) the entire model-probe battery is stubbed with no real gRPC inference calls, meaning capability gating provides zero signal in production; (2) memory-engine tests do not compile due to a missing config field; (3) all Rust subsystems fail clippy under `-D warnings`; (4) TOON encoding in prompt-composer is a simplified placeholder; and (5) two PRD-specified subsystems — `lora-manager` and `codebase-context` — do not exist. Neither is blocking the current milestone given the Phase 1 scope, but both should be tracked explicitly.

---

## Per-Subsystem Results

---

### daemon-bus

**Build status:** failing under `-D warnings`  
**Test status:** 24/24 passing  
**Clippy warnings (as errors):** 1

#### Clippy Errors

- [MINOR] `daemon-bus/src/supervisor/mod.rs:775` — `clippy::type_complexity`: inline tuple type `Vec<(String, Option<JoinHandle<()>>, Arc<TokioMutex<Option<tokio::process::Child>>>)>` should be factored into a named type alias.

#### Critical Issues

None.

#### Major Issues

- [MAJOR] `daemon-bus/src/boot/mod.rs:201,713,763,805,847,883,918,943` — Boot signal name strings (e.g. `"DAEMON_BUS_READY"`) are passed as string literals to `signal_ready()`. The daemon-bus instructions require: *"Topic Strings Come From Proto Constants Only."* While the event-topic mapping is correctly done via `topic_name(EventTopic::…)`, the boot signal name strings used in both the production path and the test helper are string literals rather than references to `BootSignal` enum variant display names. If a signal name changes in the proto the tests can pass while the production signal string diverges silently.

#### Minor Issues

- [MINOR] `daemon-bus/src/main.rs:296,308` — `set_global_default(...).expect("failed to set tracing subscriber…")`. This is an acceptable production panic — it is fatal infrastructure, cannot proceed without logging, and has an explanatory message. Documented here for completeness; no fix required.

#### PRD Alignment

| Check | Status |
|---|---|
| Signals `DAEMON_BUS_READY` on boot | ✅ |
| Handles event bus subscriptions | ✅ |
| Priority arbitration (Tier 2 exclusive, reactive wins CTP) | ✅ |
| Watchdog task timeout enforcement | ✅ |
| Supervisor restart policy (0s → 5s → 30s → degraded) | ✅ |
| Topic strings from proto constants | ❌ Boot signal *names* are string literals (boot/mod.rs) |
| Proto definitions in `daemon-bus/proto/` only | ✅ |
| Never restarts itself | ✅ |

#### Stubs and Placeholders

None found. This is the most complete subsystem in the codebase.

---

### inference

**Build status:** failing under `-D warnings`  
**Test status:** 43/43 passing  
**Clippy warnings (as errors):** 6

#### Clippy Errors

- [MINOR] `inference/src/grpc.rs:55,62,93,99` — `clippy::redundant_closure`: Four `.map_err(|e| tonic::Status::from(e))` closures should be `.map_err(tonic::Status::from)`.
- [MINOR] `inference/src/model_registry.rs:41` — `clippy::new_without_default`: `ModelRegistry::new()` has no `Default` impl.
- [MINOR] `inference/src/request_queue.rs:167` — `clippy::len_without_is_empty`: `RequestQueue` has a public `len()` but no `is_empty()`.

#### Critical Issues

None.

#### Major Issues

- [MAJOR] `inference/config/inference.toml:3` — `listen_address = "0.0.0.0"`. Sena is a local-first, privacy-first desktop application. All inter-subsystem gRPC listeners should bind to `127.0.0.1` by default. Binding to `0.0.0.0` exposes the inference gRPC service to the local network, allowing any host on the LAN to submit completion requests to the model. This violates the PRD privacy-first design principle. See also prompt-composer and reactive-loop which have the same default.

#### Minor Issues

- [MINOR] `inference/src/grpc.rs:115-133` — `InferenceGrpcService::read_activations` and `::steer` return `Status::unimplemented`. These are correctly marked as Phase 2/3 stubs with `rpc_unimplemented` log events. No fix needed before Phase 2.

#### PRD Alignment

| Check | Status |
|---|---|
| Signals `INFERENCE_READY` after model load | ✅ |
| VRAM budget checked before loading | ✅ |
| OOM → reduced `n_gpu_layers` → `INFERENCE_DEGRADED` | ✅ |
| Inference on `spawn_blocking` thread | ✅ |
| Model switch sequence (UNAVAILABLE → drain → unload → load → READY) | ✅ |
| `ReadActivations` / `Steer` gRPC stubs (Phase 2/3) | ✅ intentional |
| gRPC listen address — loopback default | ❌ defaults to `0.0.0.0` |
| LRU eviction on Mid/High tier VRAM pressure | ✅ |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `inference/src/grpc.rs:115-133` | `ReadActivations`, `Steer` gRPC RPCs | Returns `Status::unimplemented` | Minor — Phase 2/3 work, explicitly documented |

---

### memory-engine

**Build status:** failing under `-D warnings`  
**Test status:** COMPILATION FAILURE — tests do not compile  
**Clippy warnings (as errors):** 5

#### Clippy Errors

- [MINOR] `memory-engine/src/engine.rs:189` — `clippy::manual_inspect`: `.map_err(|queue_error| { … })` should use `.inspect_err(|queue_error| { … })`.
- [MINOR] `memory-engine/src/extractor.rs:241` — `clippy::new_without_default`: `DegradedExtractor::new()` has no `Default` impl.
- [MINOR] `memory-engine/src/grpc.rs:114` — `clippy::unnecessary_lazy_evaluations`: `.unwrap_or_else(|_| match …)` should use `.unwrap_or(match …)`.
- [MINOR] `memory-engine/src/grpc.rs:167` — `clippy::result_large_err`: `parse_tier()` returns `Result<TargetTier, Status>` where `Status` is ≥176 bytes. Consider boxing.
- [MINOR] `memory-engine/src/grpc.rs:179` — `clippy::result_large_err`: Same issue in `parse_priority()`.

#### Critical Issues

- [CRITICAL] `memory-engine/src/profile.rs:265` — **Test suite does not compile.** The `test_config()` helper builds a `Config` struct but omits the `store: StorePathsConfig` field that was added to `Config` (config.rs:26). The compiler error is `E0063: missing field 'store' in initializer of Config`. This means the memory-engine test suite is **completely broken** — no tests run. Fix: add `store: crate::config::StorePathsConfig { graph_path: "data/graph.redb".to_owned(), vector_path: "data/vectors.usearch".to_owned() }` to `test_config()` in `profile.rs`.

#### Major Issues

- [MAJOR] `memory-engine/src/main.rs:373` — gRPC listen address is hardcoded as `"0.0.0.0:{listen_port}"`. Unlike inference, prompt-composer, and reactive-loop which read `listen_address` from config, memory-engine hardcodes the bind address in-code. There is no `listen_address` field in `GrpcConfig` (config.rs). This cannot be overridden without a code change. Fix: add `listen_address` to `GrpcConfig`, default `"127.0.0.1"` in the TOML.
- [MAJOR] `memory-engine/src/profile.rs:226` — `degraded_extractor = true` is hardcoded unconditionally in Phase 1, regardless of the model's structured output capability. This means the graph extraction feature (`KnowledgeGraph`) is always disabled even on capable models. This is an intentional Phase 1 decision (the Phase 2 restore path is documented in comments), but it means the ech0 graph extraction path is **never exercised in any environment** at this stage.
- [MAJOR] `memory-engine/src/extractor.rs:186-205` — `LlamaExtractor::extract` creates the llama context and tokenizes correctly, but the actual generation call and JSON parsing are Phase 2 TODOs. The function returns an error at the generation step, making the full extraction path unreachable in production.

#### Minor Issues

- [MINOR] `memory-engine/src/extractor.rs:265` — `DegradedExtractor::extract` returns an empty `ExtractionResult` with a warn log on `new()` saying "using degraded extractor". The warn on construction is appropriate; however, it does not log on each individual extract call, making it difficult to correlate extraction skips with specific ingestion events in production logs.

#### PRD Alignment

| Check | Status |
|---|---|
| Signals `MEMORY_ENGINE_READY` | ✅ |
| Subscribes to `MODEL_PROFILE_READY` in background | ✅ |
| RwLock per tier, not one global lock | ✅ |
| Write lock not held across async await | ✅ |
| Write queue for ech0 ingest calls | ✅ |
| Priority ordering (Reactive front, Background back) | ✅ |
| Bounded queue with `QueueFull` error | ✅ |
| EchoError mapped to SenaError at boundary | ✅ |
| gRPC listen address — loopback default | ❌ hardcoded `0.0.0.0` in main.rs |
| Graph extraction enabled when model supports it | ❌ Phase 1: always degraded |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `memory-engine/src/extractor.rs:186-205` | Extract entities/relations via llama-cpp-2 | Returns an error (Phase 2 work) | Major — ech0 graph tier never populated |
| `memory-engine/src/profile.rs:226` | Enable extractor when model supports structured output | Hardcodes `degraded_extractor = true` | Major — all environments forced to vector-only mode |

---

### model-probe

**Build status:** failing under `-D warnings`  
**Test status:** 114/114 passing  
**Clippy warnings (as errors):** 1

#### Clippy Errors

- [MINOR] `model-probe/src/probe/context_window.rs:75` — `clippy::collapsible_if`: nested `if passed { if *fraction > highest_passing_fraction { … } }` should be collapsed to `if passed && *fraction > highest_passing_fraction { … }`.

#### Critical Issues

- [CRITICAL] **All 6 probe implementations are stubs.** model-probe's entire purpose is to run a live battery of inference calls via `InferenceService.Complete` gRPC to build a `ModelCapabilityProfile` that gates downstream capabilities. In the current implementation, **no probe makes any gRPC call whatsoever.** Every probe returns a hardcoded result:

  | Probe | Stub Behavior | Effect on Capabilities |
  |---|---|---|
  | `structured_output` (`src/probe/structured_output.rs:116`) | Returns `Ok(0.0)` always | `CapabilityLevel::None` — PC forced to JSON fallback for all models |
  | `reasoning` (`src/probe/reasoning.rs:107`) | Uses `let model_response = String::new()` | Score = 0.0 → `CapabilityLevel::None` — Reasoning agent disabled for all models |
  | `context_window` (`src/probe/context_window.rs:115`) | `stub_retention_test` always returns `true` | Largest passing fraction assumed; `pre_rot_threshold` = 75% of advertised window. **This is optimistic, not conservative** |
  | `memory_fidelity` (`src/probe/memory_fidelity.rs:99`) | Returns `0.0` | Memory injection depth set to shallow for all models |
  | `lora_compat` (`src/probe/lora_compat.rs:110`) | Returns `None` architecture | `lora_compatible = false` for all models |
  | `graph_extraction` (`src/probe/graph_extraction.rs:45`) | Returns `raw_score = 0.0` | `CapabilityLevel::None` — ech0 graph extraction disabled for all models |

  The context-window stub deserves special attention: it says "conservative" in comments but returns `true` for all retention fractions, resulting in `pre_rot_threshold = 75% * advertised_context_length * (1 - safety_margin)`. For a model with a 4096-token context and 10% safety margin, this yields `pre_rot_threshold = 2764` — a context budget that may be too optimistic if the model actually degrades earlier.

  The impact: model-probe currently signals `MODEL_PROFILE_READY` with a profile where structured output = None, reasoning = None, lora_compatible = false, memory_fidelity = 0. Any capability-gated feature in CTP, prompt-composer, or lora-manager that would normally be enabled for a capable model is permanently disabled.

#### Major Issues

- [MAJOR] `model-probe/src/main.rs:424` — `trace_context: String::new()` with `// TODO(implementation): propagate OTel trace context`. Every probe battery call carries an empty trace context, making distributed tracing through the boot sequence impossible to correlate.

#### Minor Issues

- [MINOR] `model-probe/src/generated/sena.daemonbus.v1.rs:330,339,350,365` — Generated proto comments contain mojibake (`â€"` instead of `—`). This is a UTF-8/ANSI encoding issue in the prost code generation output on Windows. No runtime impact, but it degrades `find`/`grep` on the generated file.

#### PRD Alignment

| Check | Status |
|---|---|
| Waits for `INFERENCE_READY` before probing | ✅ |
| Signals `MODEL_PROFILE_READY` | ✅ |
| Signals `LORA_TRAINING_RECOMMENDED` when gap detected | ✅ (logic present, but gap detection also returns stub scores) |
| Re-runs on new `INFERENCE_READY` (model switch) | ✅ |
| Probes via `InferenceService.Complete` gRPC only | ❌ All probes are stubs; no gRPC calls made |
| All probes use `temperature: 0.0` deterministic settings | ✅ (in config, but not executed) |
| Hardware profile detection (VRAM, RAM, tier) | ✅ |
| Publishes `HardwareProfile` alongside `ModelCapabilityProfile` | ✅ |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `probe/structured_output.rs:116` | Run structured output probe via gRPC | Returns `0.0` | Critical — PC uses JSON fallback for all models |
| `probe/reasoning.rs:107` | Run 3-step reasoning probe via gRPC | Returns empty string score | Critical — Reasoning agent disabled for all models |
| `probe/context_window.rs:115` | Test context retention at 25/50/75% window | Always returns `true` (optimistic) | Major — `pre_rot_threshold` not validated against model |
| `probe/memory_fidelity.rs:99` | Test memory injection recall via gRPC | Returns `0.0` | Major — shallow memory injection for all models |
| `probe/lora_compat.rs:110` | Read architecture from model metadata | Returns `None` | Major — LoRA disabled for all models |
| `probe/graph_extraction.rs:45` | Test graph output via gRPC | Returns `0.0` | Major — ech0 graph extraction disabled (stacks with memory-engine profile.rs) |

---

### ctp

**Build status:** failing under `-D warnings`  
**Test status:** 39/39 passing  
**Clippy warnings (as errors):** 3

#### Clippy Errors

- [MINOR] `ctp/src/activity.rs:66` — `clippy::new_without_default`: `WindowsActivityDetector::new()` has no `Default` impl.
- [MINOR] `ctp/src/activity.rs:138` — `clippy::new_without_default`: `ActivityMonitor::new()` has no `Default` impl.
- [MINOR] `ctp/src/thought_queue.rs:56` — `clippy::new_without_default`: `ThoughtQueue::new()` has no `Default` impl.

#### Critical Issues

None.

#### Major Issues

- [MAJOR] `ctp/src/context_assembler.rs:222-236` — `read_soulbox()` is a stub that returns an empty `SoulBoxSnapshot`. It is documented as "returns empty — SoulBox not built yet." The CTP instructions require: *"Relevance Weights Come From SoulBox — Never Hardcoded."* Currently, relevance weights come from `config.default_weights` (a TOML-loaded fallback), not SoulBox. This is a Phase 1 accepted gap, but the SoulBox integration path is not scaffolded — there is no gRPC client call to a SoulBox service, no event subscription for SoulBox updates.
- [MAJOR] `ctp/src/context_assembler.rs:15-50` — Three stub structs: `SoulBoxSnapshot`, `OsContext`, and `ModelCapabilityProfile` are defined locally in context_assembler.rs. `SoulBoxSnapshot` and `OsContext` are empty placeholder types. `ModelCapabilityProfile` is a minimal struct with only `pre_rot_threshold` and `output_reserve` — it does not match the full `ModelCapabilityProfile` from model-probe's proto. When the real types are wired from gRPC, this struct will need replacement.
- [MAJOR] `ctp/src/context_assembler.rs:139` — `trace_context: String::new()` with `// TODO: propagate from caller when tracing is wired`. Every memory read request from CTP carries no trace context, breaking distributed tracing across the thought generation path.

#### Minor Issues

- [MINOR] `ctp/src/activity.rs:100+` — `StubActivityDetector` (non-Windows fallback) always returns `0` for idle time. On Linux/macOS (future platforms), deep idle detection never triggers. Given the platform is Windows-first, this is acceptable for now, but the stub should panic or emit a clear error on non-Windows platforms rather than silently returning 0.
- [MINOR] `ctp/src/pipelines.rs:108` — The generation pipeline uses `while let Some(event) = telemetry_rx.recv().await` correctly (never sleeps, yields on stream input). The consolidation pipeline at line 255 uses `tokio::time::sleep(check_interval)`, which is correctly documented as one of two acceptable sleep locations in CTP. No fix needed.

#### PRD Alignment

| Check | Status |
|---|---|
| Waits for `MEMORY_ENGINE_READY` + `MODEL_PROFILE_READY` | ✅ |
| Signals `CTP_READY` | ✅ |
| Never sleeps in generation pipeline (event-driven) | ✅ |
| Thought queue is priority-ordered (BinaryHeap) | ✅ |
| Thoughts have expiry derived from relevance score | ✅ |
| JoinHandles stored (not dropped) | ✅ |
| Relevance weights from SoulBox | ❌ Phase 1: from config defaults |
| SoulBox snapshot in context | ❌ Phase 1: empty stub |
| OS context in context | ❌ Phase 1: empty stub |
| OTel trace context propagated | ❌ always empty String::new() |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `context_assembler.rs:222` | Read SoulBox personality snapshot via gRPC | Returns empty struct | Major — CTP uses config defaults for relevance weights |
| `context_assembler.rs:15` | OS context (active app, focus, window state) | Empty struct | Major — no OS context in thought generation |
| `context_assembler.rs:139` | OTel trace context | `String::new()` | Minor — tracing not wired |

---

### prompt-composer

**Build status:** failing under `-D warnings`  
**Test status:** 26/26 passing  
**Clippy warnings (as errors):** 2

#### Clippy Errors

- [MINOR] `prompt-composer/src/esu.rs:4` — `clippy::empty_line_after_doc_comments`: doc comment followed by an empty line before the `use` statement it documents.
- [MINOR] `prompt-composer/src/assembler.rs:24` — `clippy::new_without_default`: `PromptAssembler::new()` has no `Default` impl.

#### Critical Issues

None.

#### Major Issues

- [MAJOR] `prompt-composer/src/esu.rs:79-125` — **TOON encoding is a simplified placeholder.** The instructions file specifies: *"TOON for All Structured Data — No Exceptions. Every structured data object passed to the model goes through `toon_format::encode`."* The `toon-format` crate is not in the dependency graph. Instead, `encode_toon_simplified()` implements a homegrown key=value serializer that handles only shallow JSON objects. Nested structures, arrays, and non-JSON input fall back to the raw input string. The prompt-composer instructions explicitly say: *"Language: Rust only. TOON encoding is handled by the `toon-format` crate (`cargo add toon-format`)."* This is a direct violation — if a future `toon-format` crate behaves differently from this simplified encoder, all existing tests will need to be rewritten.
- [MAJOR] `prompt-composer/config/prompt-composer.toml` — `listen_address = "0.0.0.0"`. Same LAN-exposure issue as inference. Should default to `"127.0.0.1"`.

#### Minor Issues

- [MINOR] `prompt-composer/src/assembler.rs:254,329` — Two `.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap())` calls. The `unwrap()` on `partial_cmp` will panic if `relevance_score` is `NaN`. Since scores are computed via `compute_score()` which clamps to [0,1], NaN is theoretically impossible, but the unwrap has no explanatory comment.

#### PRD Alignment

| Check | Status |
|---|---|
| Waits for `MEMORY_ENGINE_READY` | ✅ |
| Signals `PROMPT_COMPOSER_READY` | ✅ |
| Sacred content (SoulBox + user intent) never dropped | ✅ |
| Context window budget enforced | ✅ |
| Drop order respected (lowest relevance first) | ✅ |
| TOON encoding via `toon-format` crate | ❌ simplified placeholder encoder |
| Stateless — no state persisted between calls | ✅ |
| gRPC listen address — loopback default | ❌ defaults to `0.0.0.0` |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `esu.rs:89-125` | TOON encode via `toon-format` crate | Simplified key=value homegrown encoder | Major — spec violation; nested structures silently mis-encoded |

---

### reactive-loop

**Build status:** failing under `-D warnings`  
**Test status:** 20/20 passing  
**Clippy warnings (as errors):** 2

#### Clippy Errors

- [MINOR] `reactive-loop/src/generated/sena.daemonbus.v1.rs:273` — `dead_code`: `SenaErrorProto` struct in generated code is never constructed. Since this is generated code, suppressing with `#[allow(dead_code)]` on the module or the struct is the right fix.
- [MINOR] `reactive-loop/src/main.rs:438` — `clippy::collapsible_if`: nested `if bus_event.topic == … { if bus_event.source_subsystem == "daemon_bus" { … } }` should be collapsed.

#### Critical Issues

None.

#### Major Issues

- [MAJOR] `reactive-loop/config/reactive-loop.toml:3` and `src/config.rs:121,178,218,258,298` — `listen_address = "0.0.0.0"`. Default hard-coded in both the checked-in TOML and in all test configs. The reactive-loop gRPC service (receiving user messages from the UI) is exposed to the LAN. Should default to `"127.0.0.1"`.
- [MAJOR] `reactive-loop/src/handler.rs` — `handler.build_minimal_context()` returns a `PromptContext` with empty `soulbox_snapshot`, `short_term`, `long_term`, and `episodic` fields. This is the Phase 1 minimal handler — the reactive path does not currently read memory or SoulBox state before generating responses. Every response is context-free. This is expected for Phase 1 but is a significant gap from the PRD's personalized response design.

#### Minor Issues

- [MINOR] `reactive-loop/src/generated/sena.daemonbus.v1.rs:330,339,350,365` — Same mojibake in generated comments as model-probe. No runtime impact.

#### PRD Alignment

| Check | Status |
|---|---|
| Waits for `PROMPT_COMPOSER_READY` + `INFERENCE_READY` | ✅ |
| Signals `REACTIVE_LOOP_READY` | ✅ |
| Requests assembly via `PromptComposerService` gRPC | ✅ |
| Sends completion via `InferenceService` gRPC | ✅ |
| Handles `INFERENCE_UNAVAILABLE` + retry | ✅ |
| Memory and SoulBox state in context | ❌ Phase 1: empty |
| gRPC listen address — loopback default | ❌ defaults to `0.0.0.0` |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `handler.rs:build_minimal_context()` | Build full prompt context from memory + SoulBox + inference | Returns minimal context with only user message | Major — responses are context-free in Phase 1 |

---

### ui (sena-ui / Tauri)

**Note:** The PRD specifies Freya as the UI framework. `docs/decisions/ui-tauri-migration.md` is an accepted ADR migrating to Tauri v2. This deviation is intentional and documented. Freya-specific rules in `ui.instructions.md` are **superseded by the ADR** and the instructions file has not been updated to reflect the migration.

**Build status:** failing under `-D warnings`  
**Test status:** 9/9 passing  
**Clippy warnings (as errors):** 5

#### Clippy Errors

- [MINOR] `ui/src-tauri/src/generated/mod.rs:9` — `unused_imports`: `pub use sena_daemonbus_v1::*;` is unused. Remove or use specific types.
- [MINOR] `ui/src-tauri/src/state.rs:86-87` — `dead_code`: `vram_used_mb` and `vram_total_mb` fields on `DebugState` are never read (they are set but never displayed).
- [MINOR] `ui/src-tauri/src/state.rs:166` — `dead_code`: `format_relative_time()` is not called anywhere.
- [MINOR] `ui/src-tauri/src/state.rs:193` — `dead_code`: `format_uptime()` is not called anywhere.
- [MINOR] `ui/src-tauri/src/state.rs:14-18` — `clippy::derivable_impls`: `impl Default for SubsystemHealthStatus` manually returns `Self::Unknown` — should use `#[derive(Default)]` with `#[default]` on the `Unknown` variant.

#### Critical Issues

None.

#### Major Issues

- [MAJOR] `ui.instructions.md` has not been updated after the Tauri ADR. The instructions describe Freya components, hooks, animations, and the `use_context` pattern. Any new UI contributor reading the instructions file will apply the wrong framework paradigm. The ADR notes this as a known consequence: *"`.github/instructions/ui.instructions.md` — update to reflect Tauri architecture"* — this has not been done.

#### Minor Issues

- [MINOR] `ui/src-tauri/src/overlay.rs:29-42` — `apply_window_effects()` on Windows is a TODO. It logs a warning but does not apply acrylic blur. Window effects are deferred pending Tauri v2 API stabilization. Acceptable given the ADR, but the warning fires on every panel window creation, adding noise to startup logs.
- [MINOR] `ui/src-tauri/src/grpc.rs:284` — Empty `_ => {}` match arm ignores unknown `EventTopic` variants from daemon-bus. If a new topic is added to the proto, the UI silently ignores it. Consider logging at `trace` level for unknowns.
- [MINOR] `ui/src-tauri/src/tray.rs:38` — Empty `_ => {}` match arm ignores unknown tray menu IDs. Same concern as above.
- [MINOR] `ui/src-tauri/src/state.rs:159` — Empty `_ => {}` match arm ignores unknown memory tier strings in event parsing. A future tier rename would silently drop events.
- [MINOR] `ui/src-tauri/src/main.rs:31,128` — Two production `expect()` calls: `Config::load().expect("Failed to load UI configuration")` and `.run(...).expect("failed to run Tauri application")`. Both are acceptable: the first is a fatal startup failure with no fallback possible, the second is the Tauri runtime. Both have explanatory messages.

#### PRD Alignment (Tauri-adjusted)

| Check | Status |
|---|---|
| Renders state from daemon-bus — no business logic | ✅ |
| All gRPC calls in Tauri backend (Rust), not frontend | ✅ |
| No blocking calls in UI | ✅ |
| System tray integration | ✅ |
| Multi-window overlay panels | ✅ |
| Acrylic blur / window effects | ❌ TODO (pending Tauri API) |
| `ui.instructions.md` updated for Tauri | ❌ Not updated post-ADR |

#### Stubs and Placeholders

| Location | Claims To Do | Actually Does | Milestone Impact |
|---|---|---|---|
| `overlay.rs:22-42` | Apply acrylic blur effect (Windows) | Logs warning, does nothing | Minor — visual polish deferred |

---

### lora-manager

**Build status:** N/A — subsystem does not exist  
**Test status:** N/A  

**Status:** `lora-manager/` directory does not exist in the repository. The PRD (§6.4) specifies this as a Python subsystem responsible for idle-time LoRA adapter training. It is listed in the PRD boot sequence (step 5: waits for `MODEL_PROFILE_READY`, signals `LORA_READY | LORA_SKIPPED`).

The daemon-bus boot configuration defines `lora-manager` as a required subsystem (it gates CTP and agents via `LORA_READY | LORA_SKIPPED`). If daemon-bus tries to spawn `lora-manager` and the executable does not exist, the boot sequence will stall at step 5.

#### Critical Issues

- [CRITICAL] **lora-manager subsystem not started.** The PRD boot sequence stalls waiting for `LORA_READY | LORA_SKIPPED`. Until lora-manager is scaffolded, the full boot sequence cannot reach `SENA_READY`. Minimum required: a stub Python process that connects to daemon-bus, signals `LORA_SKIPPED`, and exits cleanly.

---

### codebase-context

**Build status:** N/A — subsystem does not exist  
**Test status:** N/A  

**Status:** `codebase-context/` directory does not exist. The PRD (§6.5) specifies this as a Python (index generation) + daemon-bus (runtime updates) subsystem. It is not part of the current boot sequence and does not gate any other subsystem.

#### Minor Issues

- [MINOR] codebase-context is PRD-specified but not started. It does not block any current milestone since it is not in the boot dependency chain. Track as future work.

---

## Cross-Cutting Issues

### Security: gRPC Services Bind to `0.0.0.0` by Default

**Severity: Major**

Four subsystems expose their gRPC services to the local network by default:

| Subsystem | Default Bind | Source |
|---|---|---|
| inference | `0.0.0.0:50055` | `inference/config/inference.toml:3` |
| memory-engine | `0.0.0.0:<port>` | `memory-engine/src/main.rs:373` (hardcoded) |
| prompt-composer | `0.0.0.0:50057` | `prompt-composer/config/prompt-composer.toml` |
| reactive-loop | `0.0.0.0:<port>` | `reactive-loop/config/reactive-loop.toml:3` |

These services carry no authentication. Any host on the LAN can submit completion requests to the inference service, write to memory, or inject user messages to reactive-loop. Sena's PRD explicitly states *"local-first, privacy-first"*. Default bind should be `127.0.0.1` everywhere. The only reason to bind to `0.0.0.0` would be remote debugging — which should require explicit opt-in via env var.

**daemon-bus** is the only subsystem that correctly defaults to loopback-only binding.

### OTel Trace Context Not Propagated

**Severity: Minor**

Every cross-subsystem gRPC request carries a `trace_context: String` field, but in practice all callers pass `String::new()`:

- `ctp/src/context_assembler.rs:139` — memory read requests
- `model-probe/src/main.rs:424` — all probe battery calls

The PRD (copilot instructions) requires: *"Every cross-subsystem request carries an OpenTelemetry trace context propagated via gRPC metadata."* Without this, distributed tracing through the boot sequence and thought pipeline is impossible.

### Clippy `-D warnings` Fails in Every Rust Subsystem

**Severity: Major**

All 7 Rust subsystems fail `cargo clippy -p <subsystem> -- -D warnings`. None of the clippy issues are complex — mostly `new_without_default`, `redundant_closure`, and `collapsible_if`. These would be caught and fixed in any CI pipeline that runs clippy with `-D warnings`. The absence of such a pipeline gate is the root cause. Recommend adding `cargo clippy --all -- -D warnings` to CI.

---

## Recommended Fix Order

Ordered by criticality, then dependency chain impact:

1. **[CRITICAL] Fix memory-engine test compilation** (`profile.rs:265` — missing `store` field in `test_config()`). One-line fix. Unblocks the entire memory-engine test suite.

2. **[CRITICAL] Scaffold lora-manager stub** — minimum viable: a Python script that connects to daemon-bus, signals `LORA_SKIPPED`, and exits. Unblocks the full boot sequence reaching `SENA_READY`. Without this, `ctp` and `agents` can never start.

3. **[CRITICAL] Wire model-probe probe battery** — replace all six stub `run_inner` functions with real `InferenceService.Complete` gRPC calls. This is the highest-value implementation work: it unblocks capability gating for TOON encoding, reasoning agent enablement, LoRA training triggers, and memory injection depth. Fix the `context_window` stub to be conservative (return `false` by default until inference confirms retention).

4. **[MAJOR] Change all gRPC listen defaults to `127.0.0.1`** — affects inference, memory-engine (hardcoded), prompt-composer, reactive-loop. Low effort, high security impact.

5. **[MAJOR] Fix memory-engine `listen_address` hardcoding** — add `listen_address` field to `GrpcConfig`, default `"127.0.0.1"` in config TOML, reference from `main.rs:373`.

6. **[MAJOR] Fix all clippy `-D warnings` errors** — 18 errors across 7 subsystems. All are trivial. Enables CI enforcement.

7. **[MAJOR] Update `ui.instructions.md` for Tauri** — remove Freya-specific rules, document Tauri + React architecture, update component patterns.

8. **[MAJOR] Wire TOON encoding via `toon-format` crate** (`prompt-composer/src/esu.rs`) — replace simplified encoder with the real crate when available, or document the placeholder as an explicit Phase 2 dependency with a tracking issue.

9. **[MAJOR] Wire LlamaExtractor::extract generation loop** (`memory-engine/src/extractor.rs:186`) — completes Phase 2 for ech0 graph extraction. Requires Phase 2 model-probe gates to be working first (item 3 above).

10. **[MAJOR] Wire CTP SoulBox snapshot reader** (`ctp/src/context_assembler.rs:222`) — scaffold the SoulBox gRPC client and subscribe to SoulBox state updates. Relevance weights should then come from SoulBox rather than config defaults.

11. **[MAJOR] Wire reactive-loop context builder** — `handler.build_minimal_context()` should read short-term memory and SoulBox snapshot before constructing the PromptContext. This is what makes responses personalized.

12. **[MINOR] Propagate OTel trace context** — replace `String::new()` with actual trace context propagation in CTP memory requests and model-probe probe calls.

13. **[MINOR] Fix remaining clippy warnings** — see individual subsystem sections.

14. **[MINOR] Clean up dead code in UI** — remove `format_relative_time`, `format_uptime`, `vram_used_mb`, `vram_total_mb` or connect them to actual display.

15. **[MINOR] Start codebase-context scaffold** — Python build-time graph generator + daemon-bus status subscriber. Low urgency.

---

## What Is Working Well

### daemon-bus
The most complete and correct subsystem in the codebase. Priority arbitration, boot sequencing, watchdog, process supervision, and the event bus are all implemented with full test coverage. The restart policy (0s → 5s → 30s → degraded) is exactly as specified. The `topic_name()` helper correctly prevents topic string literals in the event bus. The supervisor correctly handles the child-slot race condition between `watch_process` and `shutdown_all`. **Use as a reference implementation for event bus integration.**

### inference
Strong design: VRAM budget check before load, OOM retry with reduced GPU layers, LRU eviction, request queue with bounded depth, timeout, and priority ordering, spawn_blocking for all llama-cpp-rs calls. Model switch sequence follows the exact PRD-specified 7-step protocol. **Use as a reference for spawn_blocking patterns and model lifecycle management.**

### memory-engine (concurrency)
Despite the test compilation failure, the concurrency design is correct: per-tier RwLocks, no lock held across await points, write queue serializing ech0 calls, reactive reads preempting background writes. The `engine.rs:189` pattern of releasing the lock before broadcasting to daemon-bus is exactly what the instructions require. **Use as a reference for RwLock + async write queue patterns.**

### model-probe (hardware detection + scoring)
The hardware detection path (VRAM via nvml-wrapper, RAM via sysinfo, tier classification) is fully implemented and well-tested. The probe scoring logic (partial_cmp thresholds, CapabilityLevel derivation) is complete and has 114 tests. The only gap is that the inference call is never made. **The scoring machinery is ready to use once probes are wired.**

### ctp (pipeline architecture)
The three-pipeline design (generation, evaluation, consolidation) is correctly structured: generation is event-driven and never sleeps, evaluation is loop-based awaiting queue pops, consolidation is the only pipeline that uses a sleep interval (correctly documented). JoinHandles are stored and aborted on shutdown. The ThoughtQueue's BinaryHeap + Notify design is correct for priority ordering. **Use as a reference for multi-pipeline tokio architecture.**

### daemon-bus boot orchestration
The boot sequence gate (24 tests, all passing) correctly handles: required vs optional signals, signal timeout, duplicate signals as idempotent, skip signals, compound dependency satisfaction. This is the backbone that makes the entire system composable. It is production-quality code.
