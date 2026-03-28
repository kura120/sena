# Sena — Development Plan

## How This Document Works

One active milestone at a time. One active task at a time. Completed milestones are checked off and archived. New tasks cannot begin until the current task is complete. New subsystems cannot be added mid-milestone — they require an ADR and must wait for the next milestone boundary.

## Current Status

**Active Milestone:** Milestone B — Sena Can Be Spoken To
**Active Task:** Critical audit fixes (see §Critical Fixes)
**Last Completed:** Milestone A/B core implementation + Debug UI

## Critical Fixes

These are blocking issues identified by the audit reports in `docs/audit/`. They must be completed before any milestone is declared done. Complete in this order:

- [x] 1. Wire chat panel `send_message` to reactive-loop gRPC client
         — File: `ui/src-tauri/src/commands.rs`
         — Ref: `docs/audit/ui-audit.md` — "Call terminates in Tauri backend"

- [x] 2. Add `ListModels` / `LoadModel` / `UnloadModel` RPCs to InferenceService
         — File: `shared/proto/`, `inference/src/grpc.rs`
         — Ref: `docs/audit/inference-audit.md` — "Missing model management RPCs"

- [x] 3. Fix arbiter escalation expiry timer (remove `allow(dead_code)`)
         — File: `daemon-bus/src/arbitration/mod.rs`
         — Ref: `docs/audit/daemon-bus-audit.md` — "Expiry may not fire"

- [x] 4. Add `DEGRADED_MODE` broadcast after supervisor exhausts retries
         — File: `daemon-bus/src/supervisor/mod.rs`
         — Ref: `docs/audit/daemon-bus-audit.md` — "Silent degraded state"

- [x] 5. Emit `TOPIC_CONVERSATION_TURN` from reactive-loop after each turn
         — File: `reactive-loop/src/handler.rs`
         — Ref: `docs/audit/ui-audit.md` — "Conversation timeline empty"

- [x] 6. Emit `TOPIC_PROMPT_TRACE_SNAPSHOT` from prompt-composer after assembly
         — File: `prompt-composer/src/grpc.rs`
         — Ref: `docs/audit/prompt-composer-audit.md` — "PromptTrace empty"

- [x] 7. Replace context_window probe formula with real inference probe
         — File: `model-probe/src/probe/context_window.rs`
         — Ref: `docs/audit/model-probe-audit.md` — "Formula output, not actual probe"

- [x] 8. Replace graph_extraction probe formula with real inference probe
         — File: `model-probe/src/probe/graph_extraction.rs`
         — Ref: `docs/audit/model-probe-audit.md` — "Formula output, not actual probe"

- [x] 9. Migrate proto definitions from `daemon-bus/proto/` to `shared/proto/`
         — All subsystem `build.rs` files now reference `shared/proto/`
         — Legacy copy retained in `daemon-bus/proto/` for reference

## Milestone A — Sena Is Alive

**Status:** ✅ Core complete, critical fixes in progress

**Subsystems required:**
- [x] daemon-bus
- [x] inference
- [x] memory-engine
- [x] model-probe
- [x] ctp
- [x] Debug UI (Tauri v2 overlay)

**Definition of done:**
- Boot sequence reaches `SENA_READY`
- All required subsystems signal ready
- Debug UI shows live subsystem health
- CTP generates thoughts during conversation
- Memory tier receives conversation data

## Milestone B — Sena Can Be Spoken To

**Status:** ✅ Core complete, critical fixes in progress

**Subsystems required (in addition to A):**
- [x] prompt-composer
- [x] reactive-loop

**Definition of done:**
- User can type a message and receive a response
- Response uses assembled prompt context
- Prompt trace visible in debug UI
- Conversation timeline visible in debug UI
- Memory grows during conversation

## Milestone C — Sena Has Identity

**Status:** Not started

**Subsystems required:**
- [ ] soulbox (full implementation — encryption, migrations, cold start)
- [ ] tacet (persona, heart, reflection sub-agents)

**Definition of done:**
- SoulBox initializes on first boot (cold start)
- SoulBox encrypts with user password (Argon2id + AES-256-GCM)
- Tacet reads SoulBox and influences Sena's voice
- Evolution events logged and viewable in debug UI
- User can lock traits in SoulBox
- Debug UI shows SoulBox evolution events and trait delta feed

**Subsystem admission gate:**
Before soulbox or tacet code is written, ALL 8 admission requirements from ARCHITECTURE.md §4 must be met.

## Milestone D — Sena Can Act

**Status:** Not started

**Subsystems required:**
- [ ] Agent framework (Agent trait, Router, ToolLoop, SandboxEnforcer)
- [ ] platform/windows (OS hooks — file, screen, process)

**Definition of done:**
- Router agent receives tasks from reactive-loop
- File, screen, process agents execute OS tasks
- All agent actions go through platform/windows sanitization
- Capability containment enforced via OS-level restrictions
- Debug UI shows agent dispatch trace

## Milestone E — Sena Adapts

**Status:** Not started

**Subsystems required:**
- [ ] lora-manager (Python, idle-time LoRA training)
- [ ] codebase-context (build-time index + runtime status)

**Definition of done:**
- LoRA training runs during idle time
- Adapter quality gating prevents regressions
- CodebaseContext provides runtime subsystem graph
- Debug UI shows LoRA training status

## Future (No Milestone Assigned)

- macOS platform layer
- Linux platform layer
- Voice interaction (STT + TTS)
- Community agent marketplace
- SoulBox import/export
- LoRA injectable architecture (High tier hardware)
- Doc-to-LoRA integration (Sakana AI)
- sena-agent-sdk (public crate for community agents)
- Full SoulBox evolution engine
- CTP priority escalation system (full)
- Voice interaction (Whisper STT + Kokoro/Piper TTS)
- Ambient overlay UI
- Dedicated app window
- Extended OS integrations (browser, peripherals, camera/mic)
 Agent capability tier system
 CTP autonomy controls in SoulBox
 Full test suite across all subsystems
 lora-manager — idle-time adapter training pipeline
 codebase-context — build-time index generation + runtime status integration
 Reasoning gap detection in model-probe
 Adapter quality gating pipeline
 LoRA adapter versioning and per-model storage
 Multi-model inference registry — Mid/High tier parallel model loading
 Injectable architecture design — pending Doc-to-LoRA community adoption
 New hardware milestone (16GB VRAM / 32GB RAM) — unlocks injectable architecture V2
 macOS support
 Linux support
 Community agent marketplace — with permission model defined below
 SoulBox import/export
 Extractable subsystem packaging as standalone OSS
 Advanced model routing and optimization
 LoRA and community model artifact signing — cryptographic signatures required for any externally-sourced model or LoRA weights; unsigned artifacts rejected at load time
 Community agent supply chain hardening — dependency scanning (SBOM generation), signed release binaries, reproducible build pipeline
 Privacy policy and consent UX — explicit consent flows for per-agent permissions, telemetry opt-in dialogs, onboarding privacy disclosure
 Injectable architecture V2 — 1-3B core model fine-tuned for Sena identity, Doc-to-LoRA capability injectables, hardware tier High required
 ech0 V2 — procedural + resource memory components consumed by memory-engine
 Streaming inference + interrupt — Moshi integration, CTP on partial input, interrupt signal on confidence threshold, V2-V3 STT/TTS foundation
 AMI world model integration — replace perception layer if accessible open weights available for target model family
 ech0 V3 / full MIRIX — if project has momentum