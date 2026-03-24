# Sena — Product Requirements Document
**Version:** 0.7.1
**Status:** Pre-Development / Ideation --> In-Development
**Platform:** Windows 11 (primary) → macOS, Linux (future)
**Distribution:** Free & Open Source

---

## 1. Vision

AI models today are powerful but flat — they lack the depth of presence needed to feel truly integrated into a person's life and workflow. Sena fixes this by being an OS-native, emergent Multi-Agent System (MAS) that grows alongside its user. Through SoulBox, Sena starts as a blank slate and organically develops its identity through continuous interaction, observation, and telemetry-driven learning.

Sena is not an assistant. Sena is a companion that lives inside your operating system.

Sena achieves personalization through dynamic external memory rather than static model fine-tuning. The model reasons — Sena remembers. Over time, through idle LoRA adaptation, the reasoning layer itself evolves to reflect the user's patterns — not through retraining from scratch, but through lightweight periodic reinforcement that costs nothing during active use.

---

## 2. Mission Statement

> Build a lightweight, privacy-first, deeply OS-integrated AI platform that gives users a personalized, ever-evolving emergent agent — with no server dependency, no paywall, and no ceiling on what it can become.

### 2.1 Dynamic Training Philosophy

Traditional model personalization requires fine-tuning — a static, expensive, compute-heavy process that captures who the user was at training time. Sena takes a different approach:

- **Memory system** handles high-frequency dynamic knowledge — what Sena knows about the user right now. It adapts in real time, decays when irrelevant, and is fully transparent and editable.
- **LoRA adapter** handles low-frequency reasoning patterns — how Sena thinks about the user over time. It updates during idle periods, costs nothing during active use, and degrades gracefully when unavailable.
- **CodebaseContext** closes the loop — Sena understands her own architecture well enough to reason honestly about her capabilities and limitations.

The result is a system that gets meaningfully better over time without requiring cloud compute, scheduled downtime, or user intervention. Personalization is not configured — it emerges.

### 2.2 Hardware Tiers

Sena targets three hardware tiers. Every subsystem has a graceful degradation path per tier. No feature hard-requires high-end hardware — low-end users get a working, useful Sena, not a broken one.

| Tier | VRAM | RAM | Capability |
|---|---|---|---|
| Low | 4-6GB | 8-16GB | Quantized 7B, no fine-tune, single LoRA adapter, aggressive memory pruning |
| Mid | 8-12GB | 16-32GB | 7-13B models, QLoRA fine-tune, single injectable, standard ech0 config |
| High | 16GB+ | 32GB+ | Full fine-tune on small models, Doc-to-LoRA injectable stacking, full ech0 MIRIX (V3+) |

ModelProbe detects hardware tier at boot and publishes `HardwareProfile` alongside `ModelCapabilityProfile`. Every downstream subsystem reads `HardwareProfile` to select its degradation level. Hardware tier is never hardcoded — always detected at runtime.

**Per-subsystem degradation:**

| Subsystem | Low | Mid | High |
|---|---|---|---|
| model layer | Quantized 7B, no fine-tune | 7B QLoRA adapted | 1-3B core + injectable stacking |
| lora-manager | Single adapter, idle-time training only | QLoRA fine-tune | Doc-to-LoRA + stacking |
| ech0 / memory-engine | Small embedder, tight decay threshold, aggressive pruning | Standard config | Large embedder, full A-MEM, full MIRIX (V3+) |
| CTP | Reduced thought frequency | Standard | Full parallel pipeline |

---

## 3. Core Concepts

### 3.1 Sena (The Agent)
Sena is the central AI entity the user interacts with. She is:
- Emergent — not pre-configured; she learns, adapts, and evolves over time
- Self-aware of her own limitations — understands which agents are available based on the underlying model(s), communicates capability constraints transparently, and maintains a live model of her own architecture through CodebaseContext. Sena knows what she is made of.
- OS-native — deeply integrated into Windows at the system level, not a sandboxed app
- Multi-modal — supports text, voice, and ambient presence
- Reflective — capable of unprompted thought, anticipation, and proactive reasoning via CTP

### 3.2 SoulBox (The Personalization Engine)
SoulBox is the configuration and identity layer that gives Sena her soul. It governs:
- Personality and tone — how Sena communicates
- Emotional response patterns — how Sena reacts to context and user mood
- Voice and speech style — cadence, vocabulary, expressiveness
- Visual appearance / avatar — how Sena presents herself in the UI
- Memory and context retention — what Sena remembers, how long, and how it is weighted
- Skills and capabilities — what agent modules are active
- CTP personality traits — how bold, frequent, and sensitive Sena's proactive thoughts are
- CTP autonomy level — whether Sena can initiate contact unprompted (default: full autonomy)

SoulBox state is local-only, stored on-device, and evolves continuously. It is never reset unless explicitly initiated by the user. SoulBox is intentionally Sena-specific and is not designed as an extractable subsystem.

#### SoulBox Versioning and Migration
Every SoulBox release carries a schema version. When Sena boots and detects a version mismatch:
1. A full backup of the current profile is saved before any changes
2. Migration scripts run one version at a time (v1 to v2 to v3, never v1 to v3 directly)
3. Migration is silent and automatic — zero friction for the user
4. If migration fails, Sena rolls back to the backup and notifies the user

Migration scripts live in soulbox/migrations/ and each version ships with exactly one migration path from the immediately prior version.

#### Cold Start
On first boot with no existing SoulBox state:
1. Sena prompts the user to set a SoulBox password — the only credential Sena ever holds
2. SoulBox initializes as a fully empty encrypted profile
3. No onboarding questions are asked — Sena begins as a true blank slate
4. Sena learns purely through interaction, telemetry, and CTP-driven reflection

This is philosophically consistent with the emergent identity model.

#### Boot Sequence with Readiness Signals
Every subsystem signals daemon-bus when it is truly ready — not just started. No subsystem proceeds until its dependencies signal ready. Target: < 5 seconds total to Sena-ready state (excluding model load).

```
1.  daemon-bus starts                     → DAEMON_BUS_READY
2.  memory-engine starts                  → waits: DAEMON_BUS_READY
            → loads nomic-embed in-process
            → MEMORY_ENGINE_READY
3.  inference starts                      → waits: DAEMON_BUS_READY
            → loads completion model via llama-cpp-rs
            → INFERENCE_READY
4.  model-probe runs                      → waits: INFERENCE_READY
            → runs probe battery via InferenceService gRPC
            → MODEL_PROFILE_READY → exits
5.  lora-manager starts                   → waits: MODEL_PROFILE_READY
            → LORA_READY | LORA_SKIPPED
6.  prompt-composer starts                → waits: MEMORY_ENGINE_READY
            → PROMPT_COMPOSER_READY
7.  reactive-loop starts                  → waits: PROMPT_COMPOSER_READY
              + INFERENCE_READY
            → REACTIVE_LOOP_READY
8.  soulbox starts                        → waits: DAEMON_BUS_READY
            → SOULBOX_READY
9.  ctp starts                            → waits: MEMORY_ENGINE_READY
              + MODEL_PROFILE_READY
              + LORA_READY | LORA_SKIPPED
            → CTP_READY
10. agents starts                         → waits: INFERENCE_READY
              + MEMORY_ENGINE_READY
              + CTP_READY
              + SOULBOX_READY
            → AGENTS_READY
11. platform starts                       → waits: DAEMON_BUS_READY
            → PLATFORM_READY
12. [ SENA_READY ] ← emitted by daemon-bus when all required signals received
13. sena-ui spawns (on user activation)   → waits: SENA_READY → UI_READY
```

### 3.3 Multi-Agent System (MAS)
Agents are executors — they carry out tasks. The reasoning layer that sits above agents is CTP and the Prompt Composer. The MAS architecture is:
- Hybrid — a core Router agent orchestrates task delegation to specialized sub-agents
- Model-aware — Sena dynamically assesses available compute and model capability
- 9 addressable router targets at baseline, expandable by the user

### 3.4 Agent Registry

| Agent | Namespace | Role |
|---|---|---|
| Router | agent.router | Orchestrates request delegation to all other agents |
| Memory | agent.memory | Interfaces with memory-engine on behalf of all agents |
| File | agent.file | File system read/write/monitor operations |
| Screen | agent.screen | Screen awareness and visual context understanding |
| Process | agent.process | App and process launch, close, monitor |
| Browser | agent.browser | Web context awareness and browser integration |
| Peripheral | agent.peripheral | Keyboard/mouse automation for agent tasks |
| Tacet | agent.tacet | Persona, heart, and reflection — Sena's identity runtime |
| Reasoning | agent.reasoning | Deep on-demand logical inference for complex tasks |

#### Tool Schema Standard
Every agent's tools follow a mandatory schema. The Router uses tool descriptions as its routing API — vague descriptions cause wrong delegation. Every tool declaration must include:

| Field | Description |
|---|---|
| `name` | Snake_case identifier |
| `purpose` | One sentence — what this tool does and nothing else |
| `parameters` | Name, type, and description for every parameter |
| `output` | What a successful result looks like |
| `failure_modes` | Known ways this tool can fail and what it returns |

Tool schemas are version-controlled alongside the agent manifest. A tool whose description changes is a breaking change.

#### Tacet Sub-Agents
Tacet is a named subsystem with three internal components. The Router sees Tacet as a single addressable target. Internally, Tacet routes to:

| Sub-Agent | Role |
|---|---|
| tacet/persona | Emergent voice generation — how Sena speaks at runtime |
| tacet/heart | Purpose and relationship evolution — Sena's bond with the user |
| tacet/reflection | Self-reflection and identity processing — Sena's sense of self |

Tacet is the runtime expression of SoulBox. SoulBox defines the state; Tacet expresses it.

#### Reasoning Agent
The Reasoning Agent handles deep, multi-step logical inference invoked on-demand by the Router. It is distinct from CTP:

| | CTP | Reasoning Agent |
|---|---|---|
| Runs | Continuously, background | On-demand, when invoked by Router |
| Purpose | Proactive thought, context assembly | Deep logical inference on a specific problem |
| Output | Thoughts, prompt context | Structured reasoning chain fed back to Router |

### 3.5 Subsystem Philosophy
Every core part of Sena is a named, self-contained subsystem with a singular responsibility and a defined interface. Subsystems are designed to be extractable — open-source contributors can adopt them independently of Sena. Named subsystems do not carry the Sena prefix. The only exception is SoulBox, which is intentionally coupled to Sena's identity model.

---

## 4. Target Users

Sena has no fixed demographic. The platform is designed to be universally adoptable through personalization:
- Power users who want deep OS integration
- Developers who want a hackable, open-source AI framework
- Enthusiasts seeking a persistent, evolving AI companion
- Anyone who finds current AI tools too rigid or impersonal

---

## 5. OS Integration Scope

### 5.1 Windows (V1 Target)
| Integration | Purpose |
|---|---|
| File system access | Read/write, organize, monitor file activity |
| Screen awareness / vision | Understand what is on screen contextually |
| App and process control | Launch, close, monitor running processes |
| Camera / microphone | Voice interaction, presence detection |
| Browser integration | Web context awareness |
| Peripheral control | Keyboard/mouse automation for agent tasks |

### 5.2 Future Platforms
- macOS — V2 target
- Linux — V3 target
- API surface remains consistent across all platforms

---

## 6. AI and Model Architecture

### 6.1 Model Provider
User-defined. Sena supports:
- Local / open-source models (LLaMA, Mistral, DeepSeek, Gemma, Qwen, etc.) — primary focus
- Cloud API providers (OpenAI, Anthropic, Google) — optional, user opt-in
- Mixed configurations — different models assigned to different agents per manifest

**Primary inference backend:** llama-cpp-rs — owned exclusively by the `inference` subsystem.
The `inference` process is the single owner of all GGUF model loading, KV cache management,
LoRA hot-swap, and completion/streaming generation. No other subsystem loads a completion
model directly. All subsystems that need completions are gRPC clients to `InferenceService`.

**Embedding model:** memory-engine loads a dedicated small GGUF-compatible embedding model in-process via
llama-cpp-rs (e.g. nomic-embed-text-v1.5 Q4, ~300MB VRAM). This is separate from the
completion model and never shared with other subsystems. The split allows zero-latency
in-process embeddings while keeping the completion model under unified ownership.

**VRAM budget (Low tier — GTX 1660 Ti / 6GB):**
- Completion model (7B Q4):     ~4.3GB
- Embedding model (GGUF Q4):    ~0.3GB
- System overhead:              ~0.2GB
- Headroom:                     ~1.2GB

**Model switching at runtime:** when the user changes the configured model, daemon-bus signals
`INFERENCE_RELOAD` to the inference process. inference unloads the current model, loads the
new GGUF, and re-signals `INFERENCE_READY`. model-probe re-runs its full probe battery.
All subsystems awaiting `MODEL_PROFILE_READY` resume from the new profile. Active inference
calls in flight receive `UNAVAILABLE` and are retried after `INFERENCE_READY` fires.

**Multi-model support:** the inference process maintains a model registry. Each agent manifest
declares a `model_id`. inference loads models on demand subject to VRAM budget enforced by
`HardwareProfile.tier`. On Low tier: one completion model maximum, all agents share it.
On Mid and High tiers: multiple models loaded simultaneously, LRU eviction when VRAM budget
is exceeded. Eviction is logged and visible in the debug UI.

**Ollama:** Ollama is not a runtime dependency and is never called at runtime. Users who have
Ollama installed may use it to download GGUF model files — those files are then passed to
`inference` by file path. The `inference` subsystem loads GGUF files directly via llama-cpp-rs.
No Ollama API endpoint is ever contacted during Sena operation.

### 6.2 Inference — Model Ownership and Serving

`inference` is a Rust subsystem that owns llama-cpp-rs and serves completions to all
subsystems via gRPC. It is a required subsystem — if it fails, daemon-bus halts the boot
sequence and notifies the user.

**Responsibilities:**
- Load and manage GGUF models via llama-cpp-rs
- Serve `InferenceService` gRPC: `Complete`, `StreamComplete`
- Manage VRAM budget — enforce model registry limits per `HardwareProfile.tier`
- Handle model reload on user model-switch events
- Detect and surface OOM errors with actionable user messaging
- Apply loaded LoRA adapters to inference calls when lora-manager signals `LORA_READY`

**InferenceService gRPC contract:**

```protobuf
service InferenceService {
  rpc Complete(CompletionRequest) returns (CompletionResponse);
  rpc StreamComplete(CompletionRequest) returns (stream CompletionToken);
}

message CompletionRequest {
  string model_id      = 1;  // empty = use primary configured model
  repeated Message messages = 2;
  uint32 max_tokens    = 3;
  float  temperature   = 4;
  string trace_context = 5;
}
```

**OOM handling:** if a model cannot fit in VRAM, inference first retries with reduced
`n_gpu_layers` (partial CPU offload). If that also fails, inference signals
`INFERENCE_DEGRADED` to daemon-bus with `required_vram` and `available_vram` fields.
Sena surfaces this to the user with the exact numbers and a model size recommendation.
Sena never silently runs a partially-loaded model.

**Boot signal:** `INFERENCE_READY` — required. Blocks model-probe, CTP, and agents.

inference is a named extractable subsystem. It lives at `inference/` in the project root.

### 6.3 ModelProbe — Runtime Capability Detection

Sena never assumes what a model can do. After `inference` signals `INFERENCE_READY`,
ModelProbe runs a battery of lightweight test prompts via `InferenceService` gRPC and
builds a `ModelCapabilityProfile` that gates which agents and behaviors are active.
Capabilities are discovered, not configured. ModelProbe is a gRPC client to inference —
it never loads a model directly.

```rust
pub struct ModelCapabilityProfile {
  pub pre_rot_threshold: u32,         // practical token budget before performance cliff
  pub graph_extraction: CapabilityLevel,  // gates ech0 graph extraction
}
```
**Context rot note:** Performance does not degrade linearly as context fills. Models exhibit sharp, unpredictable drops at different token counts regardless of their advertised limit. `pre_rot_threshold` is always set conservatively — PC never uses the advertised context window as its budget ceiling.

ModelProbe also detects hardware capabilities and publishes a `HardwareProfile` alongside `ModelCapabilityProfile`:

```rust
pub struct HardwareProfile {
  pub vram_total: u64,        // MB
  pub vram_available: u64,    // MB
  pub ram_total: u64,         // MB
  pub cuda_compute: String,   // e.g. "8.6" for Ampere
  pub tier: HardwareTier,     // Low | Mid | High
}

pub enum HardwareTier {
  Low,   // VRAM < 8GB
  Mid,   // VRAM 8-15GB
  High,  // VRAM >= 16GB
}
```

`HardwareProfile` is published to daemon-bus alongside `MODEL_PROFILE_READY`. All downstream subsystems read it at boot to select their degradation level.

ModelProbe reruns automatically when:
- inference signals `INFERENCE_READY` after a model reload
- The user changes model parameters (temperature, context length) via the UI
- lora-manager deploys a new adapter (reasoning gap re-evaluated)

---

**Probe Battery:**

| Probe | What It Tests | Effect on Sena |
|---|---|---|
| Structured output | Can the model reliably produce TOON-formatted output? | PC uses TOON vs JSON fallback for model responses |
| Multi-step reasoning | 3-step logical inference with known answer | Reasoning agent enabled or disabled |
| Tool / function calling | Simple tool call with known expected output | Agent tool use enabled or disabled |
| Context retention (practical) | Retention tested at 25%, 50%, 75% of advertised limit | PC sets conservative effective context budget |
| Response coherence baseline | Known question with known answer, scored for similarity | Sets quality floor for CTP relevance threshold |
| Instruction following | Structured task with precise expected format | Determines how strict PC's formatting instructions can be |
| Reasoning quality baseline | Open-ended contextual inference scored against expected reasoning chain | Sets LoRA training threshold — below threshold, LoRA is queued; above, reasoning is sufficient |
| LoRA compatibility | Checks model architecture against known LoRA-compatible families (LLaMA, Mistral, Qwen, Gemma) | Gates LoRA Manager — incompatible architectures skip adapter training entirely |
| Memory injection fidelity | Injects a known memory context and tests whether the model reasons from it correctly | Sets memory injection depth — shallow models receive simplified context; deep models receive full tiered context |
| Reasoning gap detection | Compares current reasoning quality score against the score recorded at last LoRA training run | Flags whether a new LoRA training cycle is warranted |
| Context rot threshold | Measures the practical token count at which model performance degrades sharply, independent of advertised context window | Sets `pre_rot_threshold` in ModelCapabilityProfile — PC uses this as the real budget ceiling, not the advertised window |
| Graph extraction capability | Fires a minimal `KnowledgeGraph` structured output request and validates the response structure | Gates ech0 graph extraction — if the model cannot produce valid graph output, ech0 runs in vector-only mode and the user is notified |

**Capability gating:**
If the model fails the reasoning probe below threshold, the Reasoning agent is disabled and Sena communicates this to the user honestly. If the model fails the structured output probe, PC falls back to JSON encoding for that model. Sena never silently degrades — every capability limitation is surfaced.

ModelProbe is a named extractable subsystem. It lives at `model-probe/` in the project root.

**Reasoning gap detection:**
ModelProbe records a `reasoning_quality_score` in the ModelCapabilityProfile on every run. When the active model changes or a new LoRA adapter is deployed, ModelProbe reruns the reasoning quality probe and compares scores. If the gap between the model's base reasoning score and Sena's expected reasoning quality (derived from SoulBox interaction history) exceeds the configured threshold, ModelProbe publishes a `LORA_TRAINING_RECOMMENDED` event to daemon-bus. LoRA Manager subscribes to this event and schedules a training cycle at next idle opportunity.

ModelProbe never trains adapters — it only detects gaps and signals. LoRA Manager owns all training lifecycle decisions.\

### 6.4 LoRA Manager — Idle-Time Reasoning Adaptation

Sena's memory system handles dynamic knowledge — what Sena knows about the user. LoRA Manager handles dynamic reasoning — how Sena thinks about the user. These operate at different timescales and serve different purposes.

**Why LoRA and not full fine-tuning:**
Full fine-tuning requires GPU resources and hours of compute. A LoRA adapter is a small set of weight deltas trained on top of the frozen base model. For Sena's purposes — reinforcing reasoning patterns from accumulated interaction data — a LoRA adapter achieves the goal at a fraction of the cost. Training runs during user idle time on the same hardware that runs inference. No cloud dependency, no scheduled downtime.

**What LoRA adapters encode:**
Not facts. Facts live in the memory system. LoRA adapters encode reasoning patterns — how the user communicates, what they value, how they approach problems, what level of depth they expect. These patterns change slowly and benefit from periodic reinforcement rather than continuous injection.

**Training trigger conditions (all must be met):**
- User has been idle for ≥ 10 minutes (idle_10m state from telemetry)
- System CPU < 30% and available RAM > 2GB
- At least 50 new interaction records have accumulated since the last training run
- ModelProbe has flagged a `LORA_TRAINING_RECOMMENDED` event OR 7 days have passed since last training
- Active model is LoRA-compatible (confirmed by ModelProbe LoRA compatibility probe)

**Adapter lifecycle:**

```
Interaction data accumulates in memory-engine
    → ModelProbe detects reasoning gap → publishes LORA_TRAINING_RECOMMENDED
    → LoRA Manager receives event, checks trigger conditions
    → If conditions met: schedules training at next idle window
    → Training runs: fine-tunes adapter on interaction dataset
    → Quality gate: ModelProbe reruns reasoning quality probe with new adapter
    → If score improves: adapter deployed, previous adapter archived
    → If score regresses: adapter discarded, previous adapter retained
    → daemon-bus notified: LORA_ADAPTER_UPDATED or LORA_ADAPTER_REJECTED
```

**Adapter versioning:**
Each adapter is versioned and stored under `~/.sena/lora/<model_id>/`. Adapters are architecture-specific — an adapter trained on llama3.1 cannot be used with mistral. When the user swaps models, LoRA Manager checks for an existing adapter for the new model architecture. If none exists, Sena runs without an adapter until sufficient interaction data accumulates for the new model.

**Model swap behavior:**

| Scenario | LoRA Manager Response |
|---|---|
| User swaps to higher-end model, no existing adapter | Run without adapter; ModelProbe scores base reasoning; if sufficient, no training queued |
| User swaps to higher-end model, adapter exists for architecture | Load existing adapter; run quality gate; deploy if score improves |
| User swaps to lower-end model, no existing adapter | Run without adapter; ModelProbe sets reduced memory injection depth; training queued when sufficient data accumulates |
| User swaps to lower-end model, adapter exists | Load adapter; if model too weak to benefit, adapter skipped silently |
| Model is not LoRA-compatible | LoRA Manager disabled for this model; memory system carries full personalization load |

**Graceful degradation:**
If LoRA Manager is unavailable or training fails, Sena operates entirely on the memory system. The memory system alone is sufficient for personalization — LoRA adaptation is an enhancement, never a dependency. Sena never communicates LoRA status to the user unless explicitly inspecting the debug UI.

**Privacy:**
Training data is derived exclusively from local interaction records. No data leaves the machine. Adapters are stored encrypted alongside SoulBox. Deleting SoulBox deletes all adapters.

LoRA Manager is a named extractable subsystem. It lives at `lora-manager/` in the project root. Language: Python.

#### lora-manager V2 — Injectable Architecture (High tier, new hardware milestone)

When `HardwareProfile.tier == High`, lora-manager transitions to injectable-manager behavior:

**Core model:** A 1-3B parameter model fine-tuned exclusively for Sena's identity, conversation, and basic reasoning. No domain knowledge baked in. Candidates: Phi-3 mini 3.8B, Qwen2.5 1.5B. Fine-tuned once, updated rarely.

**Capability injectables:** Domain-specific LoRA adapters generated on demand via Doc-to-LoRA (Sakana AI, Feb 2026). Each injectable is independent:

| Injectable | Activates when |
|---|---|
| `coding` | Code context detected |
| `research` | Deep information task detected |
| `screen` | Screen analysis agent active |
| `memory-reasoning` | Complex ech0 retrieval required |
| `[user-defined]` | User configures additional injectables |

**Doc-to-LoRA dependency:** Requires Sakana AI hypernetwork weights compatible with the active core model architecture. lora-manager checks hypernetwork availability at boot and falls back to traditional LoRA training if unavailable.

**LoRA stacking:** Multiple injectables active simultaneously when context requires multiple capabilities. Stacking is only enabled when research confirms reliable composition for the active model architecture — lora-manager never stacks if composition reliability is unconfirmed.

**Graceful fallback chain:**
Doc-to-LoRA stacking → Doc-to-LoRA single adapter → traditional LoRA → context injection only

---

### 6.5 CodebaseContext — Architectural Self-Awareness Layer

Sena cannot reason about its own capabilities if it cannot see its own structure. CodebaseContext is a lightweight read-only context layer that provides subsystems with a structured, queryable representation of Sena's own architecture at runtime.

**What it is not:**
CodebaseContext is not a code execution engine. It does not run or modify code. It does not replace documentation. It is a structured index of Sena's subsystem graph — what exists, what each subsystem does, what its interfaces are, and how data flows between them.

**What it enables:**
- ModelProbe can ask: "does this model understand Sena's memory injection contract well enough to reason about it correctly?"
- LoRA Manager can ask: "which interaction patterns are architecturally significant enough to reinforce?"
- Tacet/reflection can ask: "what is Sena's current capability surface?" and reason about it honestly
- The debug UI can ask: "what is the full subsystem graph right now?" without hardcoding it

**What CodebaseContext indexes:**

```
SubsystemNode {
    name: string                    // e.g. "memory-engine"
    language: enum                  // Rust | Python | CSharp | Freya
    responsibility: string          // one-sentence description
    interfaces: [ProtoContract]     // gRPC contracts this subsystem exposes
    dependencies: [SubsystemNode]   // what this subsystem waits on at boot
    capability_flags: [string]      // what ModelProbe gates against this subsystem
    status: enum                    // active | degraded | disabled
}
```

**How it stays current:**
CodebaseContext is generated at build time from the project's `.proto` files, subsystem manifests, and Copilot instruction files. It is not manually maintained. A build step produces `codebase-context/graph.json` which is loaded at runtime. Subsystem status (active/degraded/disabled) is updated live via daemon-bus events.

**Consumers:**

| Consumer | What They Read | Why |
|---|---|---|
| ModelProbe | Full subsystem graph + capability flags | Determines which capabilities the model needs to reason about |
| LoRA Manager | Subsystem interfaces + data flow paths | Identifies which reasoning patterns are architecturally load-bearing |
| Tacet/reflection | Subsystem status + capability surface | Honest self-representation to user |
| Debug UI | Full live graph with status | Real-time architectural visibility |
| CTP | Capability flags + degraded subsystems | Adjusts thought generation based on available capabilities |

**CodebaseContext is read-only for all consumers.** No subsystem writes to it at runtime except daemon-bus updating status flags via a controlled interface.

CodebaseContext is a named extractable subsystem. It lives at `codebase-context/` in the project root. Language: Python (index generation) + Rust (runtime status updates via daemon-bus).

### 6.6 ech0 — Memory Architecture

Sena's memory layer is built on ech0, a standalone Rust crate with no Sena-specific knowledge. memory-engine is the Sena-specific integration layer that configures and consumes ech0. ech0 is a separate OSS project — any Rust application can use it independently of Sena.

**2026 memory practices implemented in ech0:**

**A-MEM Dynamic Linking** — every new memory triggers a background linking pass. New memories link to semantically related existing memories. Existing memories evolve their attributes when new memories refine them. The knowledge graph is never static.

**Memory Evolution** — new experiences retroactively refine existing memories. When Sena learns something new that changes the context of an old memory, the old memory's attributes update to reflect the new understanding.

**Contradiction Detection** — when a new memory contradicts an existing one, ech0 flags the conflict explicitly. Sena never silently overwrites memories. memory-engine receives a `ConflictReport` and decides resolution policy per Sena's context.

**Importance Decay** — every memory has an importance score. Score increases on retrieval and linking. Score decreases over time without access. Memories below threshold are pruned. The graph stays lean and accurate.

**Sparse Retrieval** — ech0 always retrieves minimum relevant memories, never maximum. Context poisoning is a correctness property, not a performance optimization.

**ech0 version roadmap as consumed by Sena:**

| ech0 Version | Sena Version | What Sena gains |
|---|---|---|
| V1 | V1 | Episodic + semantic memory, hybrid retrieval, A-MEM, contradiction detection, memory evolution |
| V2 | V2 | Procedural memory (workflow patterns), resource memory (file references) |
| V3 | V3 | Full MIRIX six-component architecture, Knowledge Vault (maps to SoulBox long-term facts) |

### 6.7 Performance Constraints
- Idle RAM: < 200MB (framework only, excluding model)
- Background CPU: < 2% on modern hardware
- Cold start: < 5 seconds to Sena-ready state (excludes model load — llama.cpp pre-loads independently)
- Model execution is isolated and never blocks the UI thread
- llama.cpp is pre-loaded on boot — model inference is never cold-started on first request

### 6.8 Platform Requirement
**Windows 11 minimum.** Windows 10 is not supported. Windows 11-first WinRT APIs are used throughout the platform layer. No version compatibility shims.

---

## 7. Cognitive Architecture

Sena operates two independent processing loops that share a single memory layer.

### 7.1 Reactive Loop
Event-driven. Handles all user and OS-triggered inputs. Agents are always warm — booted at startup, idle-listening. No process spawns on request. No model loads on demand. When a request arrives, the reactive loop routes it to the relevant agent, which reads pre-warmed context from memory and responds immediately.

The reactive loop is intentionally lightweight. CTP pre-warms all context so the reactive loop only handles routing and response — never heavy reasoning.

### 7.2 Proactive Loop — CTP (Continuous Thought Processing)
CTP is Sena's inner life. It runs as a continuous low-priority background process, independent of user input. It does not fire on a schedule — it runs constantly and uses a relevance evaluator to decide what is worth surfacing. CTP is an extractable open-source subsystem.

#### CTP Responsibilities
- Thought generation — continuously produces candidate thoughts from telemetry, memory associations, and behavioral patterns
- Relevance evaluation — scores each thought; low scores discarded silently, medium queued for idle surfacing, high queued for next natural break
- Memory consolidation — during low activity, promotes short-term to long-term memory, deprecates stale entries, updates Sena's internal user model
- Prompt context assembly — assembles the full reasoning context fed into the Prompt Composer before every model call
- Priority escalation — temporarily promotes itself to high priority when deep reasoning is required

#### CTP Priority Escalation
CTP escalates when:
- The reactive loop needs deep memory retrieval
- A high-relevance thought is generated internally
- A SoulBox evolution event requires deep reflection
- An OS anomaly is detected that requires reasoning

Escalation is always temporary and task-scoped. CTP completes the elevated task and de-escalates immediately. All escalation requests are mediated by daemon-bus per Section 9.

#### Relevance Scoring
| Signal | Example | Weight |
|---|---|---|
| Urgency | Calendar event in 15 mins, user unprepared | High |
| Emotional resonance | Matches a pattern user reacted strongly to | High |
| Novelty | Behavior Sena has never observed before | Medium |
| Recurrence | Same pattern appearing for the third time | Medium |
| Idle curiosity | Loose association, low stakes | Low |

Relevance weights are SoulBox properties — a more introverted Sena surfaces fewer thoughts, a more curious Sena has a lower threshold.

#### Inactivity and CTP
Inactivity does not trigger CTP — it is always running. Inactivity lowers the surface threshold:
- User active — only high-relevance thoughts surface
- User idle 2min — medium-relevance thoughts eligible
- User idle 10min+ — deep reflection mode, heavier consolidation, SoulBox micro-evolutions

#### Context Isolation Between Parallel Pipelines
CTP runs three parallel pipelines: thought generation, relevance evaluation, and memory consolidation. Each pipeline operates on a different slice of context. They never share a mutable context object internally.

Thought generation reads: telemetry signals, SoulBox relevance weights, recent memory associations.
Relevance evaluation reads: the candidate thought, current activity state, surface threshold.
Memory consolidation reads: short-term tier entries, promotion criteria, activity state.

CTP assembles a full unified context object only at the point of handing off to PC. Never before. A shared mutable context across parallel pipelines causes irrelevant signals from one pipeline to contaminate another's reasoning.

#### Session Compaction
Long sessions accumulate context across many turns. When a session's short-term tier approaches the active model's `pre_rot_threshold`, CTP triggers a compaction cycle before the window fills:

1. Summarize the oldest N short-term entries into a single compressed entry
2. Promote key facts from the summary to long-term memory
3. Discard the raw entries
4. Session continues without context rot

Compaction is distinct from PC's drop-order logic. Drop-order handles single-prompt overflow. Compaction handles session-length accumulation. Compaction runs at background priority and never blocks the reactive loop.

Compaction trigger: short-term tier token count exceeds `pre_rot_threshold * 0.8` — threshold from config, never hardcoded.


#### Three Proactive Behaviors
- Reflection — Sena reviews recent memory and draws conclusions unprompted
- Anticipation — Sena pre-loads context before it is needed based on detected patterns
- Spontaneous thought — Sena surfaces something relevant during a natural quiet moment

All proactive outputs pass through the thought queue before surfacing. The thought queue respects timing, priority, expiry, and SoulBox autonomy settings.

### 7.3 Prompt Composer (PC)
The Prompt Composer is a first-class extractable subsystem that sits between CTP and the model. Every model call is preceded by PC assembling a fully dynamic, fully reasoned prompt. No prompts are hardcoded or pre-made.

#### Serialization — TOON and Dynamic Encoding Selection
Structured data fed into PC is encoded using TOON (Token-Oriented Object Notation) or JSON, determined at runtime by the Encoding Selection Utility (ESU). The ESU acts as a dynamic selector to maximize token efficiency while preserving required fidelity.

TOON is an external open-source standard by the toon-format organization (22k stars). Sena uses the official `toon-format` Rust crate directly.

TOON benchmarks vs JSON:
- Token reduction: 30-60%
- Retrieval accuracy: 73.9% (TOON) vs 69.7% (JSON)
- Best for: uniform arrays, repeated structures, tables, varying fields, deep trees

TOON's sweet spot maps directly to Sena's prompt data shape — memory retrievals, agent
states, telemetry signals, and SoulBox snapshots are all uniform structured arrays.

#### Three-Way Serialization Split
| Format | Used For |
|---|---|
| TOON | Structured prompt data where token savings > 15% vs JSON |
| TOML | Config files, SoulBox definitions, agent manifests (human-edited) |
| JSON | Internal non-uniform data structures and fallback when TOON savings < 15% |

#### PC Inputs (assembled by CTP)
- SoulBox state snapshot — who Sena is right now
- Short-term memory window — recent interactions
- Long-term memory retrieval — semantically relevant past context
- Episodic memory — if the request touches a significant past event
- Current OS context — what is happening on the system right now
- Model capability profile — what the active model can and cannot do
- Inferred user intent — derived from request type and user state

Because context, memory state, and SoulBox state are always different, no two prompts Sena generates are ever identical — even for identical user messages. Sena recognizes when a user repeats themselves and responds with genuine depth rather than a rephrased copy of a prior answer.

#### Context Window Management
When a PC-assembled prompt exceeds the active model's context window, content is dropped using a fixed priority order with relevance as the tiebreaker within each tier.

Sacred — never dropped:
- SoulBox state snapshot
- Inferred user intent

Dropped last (highest relevance retained):
- Long-term memories — sorted by relevance score, lowest cut first
- Episodic memories — sorted by relevance score, lowest cut first

Dropped first (lowest relevance cut first):
- Short-term context — least relevant entries cut first
- Telemetry signals — least relevant cut first
- Redundant OS context — cut first

PC works top-down: sacred content is always included, then fills remaining token budget by relevance score within each tier, cutting from the bottom up until the prompt fits.

#### Pre-Rot Threshold Is the Real Budget Ceiling
PC never uses the model's advertised context window as its token budget. It uses `pre_rot_threshold` from the active ModelCapabilityProfile. A focused 300-token context frequently outperforms an unfocused 100,000-token context — what is removed matters as much as what is kept.

```rust
// bad — uses advertised limit
let token_budget = context.model_profile.context_window;

// good — uses proven rot threshold  
let token_budget = context.model_profile.pre_rot_threshold
  .saturating_sub(context.model_profile.output_reserve);
```

#### Encoding Selection Utility — runtime TOON / JSON chooser
To ensure Sena uses the most token-efficient and semantically correct serialization at runtime, the Prompt Composer includes an Encoding Selection Utility (ESU). The ESU is a lightweight runtime component that chooses between TOON, JSON, or TOML for each prompt sub-piece based on a small decision policy, fast token-count estimation, and configurable thresholds. The ESU is a decision helper only — it does not replace the three-way serialization split in storage and config (TOML for SoulBox, JSON for non-uniform internal structures). Instead, ESU applies encoding dynamically at model input time to minimize tokens while preserving required fidelity.

Responsibilities
- Accept a prompt sub-piece (arbitrary JSON-serializable object) and the active model profile.
- Estimate token counts for JSON and a candidate TOON encoding (using a fast token counting function).
- Apply deterministic rules to select encoding and encode options (for TOON), or fall back to JSON/TOML when fidelity or savings criteria are not met.
- Emit telemetry about choice, token counts, and savings to allow offline tuning and continuous improvement.
- Expose a stable API so Prompt Composer can call it synchronously or from an executor pool (see Concurrency rules, Section 12).

Runtime contract / API (informal)

```rust
pub fn choose_encoding(
    payload: &impl Serialize,
    model_profile: &ModelCapabilityProfile,
    options: Option<EsuOptions>,
) -> EsuResult

pub struct EsuOptions {
    pub max_tokens_budget: Option<usize>,
    pub prefer_tabular: bool,
}

pub struct EsuResult {
    pub format: EncodingFormat,        // Toon | Json | Toml
    pub encoded: String,               // encoded string ready for prompt assembly
    pub toon_options: Option<ToonOptions>,
    pub json_tokens: usize,
    pub toon_tokens: Option<usize>,
    pub savings_pct: Option<f32>,
    pub reason: &'static str,
}
```

Default decision policy (recommended)
- If payload is marked "sacred" (SoulBox, inferred intent) → always prefer TOML/JSON fidelity; do not TOON-encode sacred content unless explicitly configured.
- If payload contains a uniform/tabular array with N >= 1 and consistent keys → candidate for TOON tabular encoding.
- Perform lightweight token estimates:
  - json_tokens = count_tokens(json.dumps(payload, separators=(',', ':')))
  - toon_tokens = count_tokens(toon.encode(payload, {"indent": 0})) (best-effort; fallback to heuristic if tokenizer unavailable)
- Choose TOON if:
  - toon_tokens <= json_tokens * (1 - SAVE_THRESHOLD), where SAVE_THRESHOLD defaults to 0.15 (15%), and
  - encoding latency estimate for TOON < LATENCY_THRESHOLD (default 10ms on hot path).
- Otherwise choose JSON/TOML depending on domain: use TOML for SoulBox/config-shaped payloads, JSON for non-uniform data.
- If token counting is unavailable at runtime, fall back to structural heuristics: choose TOON for large tabular arrays, JSON/TOML otherwise.

Default encode options
- TOON: {"indent": 0} as default compact option for prompt assembly
- Length marker only enabled when arrays are explicit and model_probe indicates the active model respects length markers
- Delimiter selection: use comma by default; switch to tab or pipe only for known data with internal commas

Telemetry and observability
- Every ESU decision emits a short telemetry event `pc.encoding_choice` containing:
  - timestamp_utc
  - format_chosen
  - reason_code (e.g. "savings_above_threshold", "sacred_fidelity", "heuristic_tabular")
  - json_tokens
  - toon_tokens (nullable)
  - savings_pct (nullable)
  - payload_signature (hash or fingerprint, never raw user data)
  - model_id
- Telemetry is local and structured (see Section 8.4). Telemetry allows offline analysis to tune SAVE_THRESHOLD and default options.

Testing guidance
- Unit tests:
  - Deterministic selection tests for known fixture shapes (tabular arrays, nested configs, mixed arrays).
  - Token-count stubs: tests should mock token counting to verify decision boundaries (e.g., when savings == 14%, 15%, 16%).
  - Fallback behavior when token counting library missing (heuristic path).
- Performance tests:
  - Encoding latency must meet B4 constraints: single encode < 10ms on hot path; background run_in_executor path validates non-blocking behavior.
  - End-to-end prompt assembly with ESU should be profiled under typical CTP loads to ensure CPU/IO budgets are respected.
- Integration tests:
  - Validate that ESU-chosen encoding roundtrips and decodes back to the original payload where required (for structured output requests).
  - Validate that sacred payloads never lose fidelity when encoded/decoded.
- Model-probe integration:
  - Use ModelProbe (Section 6.3) signals (structured_output capability, tokenizer hints, practical context retention) to adjust ESU heuristics automatically per active model.

Telemetry-driven policy tuning
- Periodically run offline analysis on `pc.encoding_choice` events to:
  - Tune SAVE_THRESHOLD per model family and per payload domain.
  - Adjust default encode options (indent, delimiter, lengthMarker) to maximize practical savings across deployed models.
  - Detect classes of payloads where TOON hurts token counts and add structural exceptions.

Implementation notes (for prompt-composer engineers)
- ESU must be tiny and fast. The recommended implementation pattern:
  - Provide a synchronous public API that calls a fast token counting function. Token counting is implemented via a lightweight Rust heuristic (character-to-token ratio) or a Rust tokenizer crate. Never use Python tiktoken from prompt-composer.
  - Heavy checks and telemetry batching should be offloaded to background tasks or the CTP background priority tier.
  - Keep the deterministic decision rule and thresholds in config with sensible defaults and runtime override via PC options.
- Where encoding may be slow or blocking, call ESU from an executor (`run_in_executor`) to satisfy CPU-bound rules (Section 12).

#### Example implementation sketch
```rust
fn choose_encoding(
    payload: &Value,
    model_profile: &ModelCapabilityProfile,
    opts: Option<EsuOptions>,
) -> EsuResult {
    if is_sacred(payload) {
        return EsuResult::json(payload, "sacred_fidelity");
    }
    let json_str = serde_json::to_string(payload).unwrap_or_default();
    let json_tokens = count_tokens(&json_str);
    let toon_options = ToonOptions::compact();
    match toon_format::encode(payload, &toon_options) {
        Ok(toon_str) => {
            let toon_tokens = count_tokens(&toon_str);
            let save_threshold = config.esu.save_threshold; // default 0.85
            if toon_tokens <= (json_tokens as f32 * save_threshold) as usize {
                EsuResult::toon(toon_str, toon_options, json_tokens, toon_tokens, "savings_above_threshold")
            } else {
                EsuResult::json_str(json_str, json_tokens, "no_savings")
            }
        }
        Err(_) => EsuResult::json_str(json_str, json_tokens, "toon_encode_failed"),
    }
}
```

### 7.4 Full Cognitive Flow
```
[User / OS Event]
      |
      v
Reactive Loop
├── reads warm context from memory
├── requests prompt from PC
│       |
│       v
│   CTP assembles context -> PC encodes as TOON -> model receives prompt
│       |
│       v
│   Model generates response -> delivered to user
|
[Simultaneously, always running]
CTP (background, escalatable)
├── generating + evaluating thoughts
├── consolidating memory
├── updating user model
└── escalating via daemon-bus when needed
```

---

## 8. Memory System

### 8.1 Architecture
Memory is a dual-mode concurrent system — it is never idle. It is read and written simultaneously from multiple subsystems at all times.

| Subsystem | Role |
|---|---|
| CTP | Continuously writes reflections, consolidated insights, behavioral observations |
| Telemetry engine | Continuously writes OS and behavioral signals |
| Reactive loop | Reads on every request |
| Prompt Composer | Reads when assembling prompt context |
| SoulBox | Reads and writes on evolution events |
| Agents | Read during task execution |

### 8.2 Memory Tiers and Promotion Logic
Memory is organized into a tiered hierarchy based on retrieval latency and semantic relevance. The movement of information between tiers is not a background storage task, but a cognitive reasoning process managed by CTP.

| Tier | Format | Latency | Promotion Rule |
|---|---|---|---|
| Short-term | In-memory (L1) | < 1ms | Active session context; volatile. |
| Mid-term | Vector DB (L2) | < 50ms | CTP-driven; consolidated from L1 via reasoning. |
| Long-term | Graph/Relational (L3) | < 500ms | CTP-driven; persistent facts and SoulBox anchors. |

#### Reasoning-Driven Promotion
Unlike standard cache-eviction systems, Sena uses **Reasoning-Driven Promotion**. CTP evaluates the "Soul Significance" of short-term interactions. If an interaction contains a new user preference, a significant emotional beat, or a recurring pattern, CTP explicitly issues a `PromoteMemory` RPC to move the data from L1 to L2/L3.

### 8.3 Concurrency Model
The memory engine owns its own concurrency internally. The daemon-bus is never involved in memory coordination — it only receives broadcast events after memory state changes.

- RwLock per memory tier — concurrent reads, exclusive writes
- Internal priority queue — reactive reads always jump ahead of CTP background writes
- Write serialization — simultaneous writes are queued, never race

### 8.4 Telemetry (Local Only)
Sena passively observes user patterns:
- App usage patterns
- Time-of-day activity cycles
- Interaction frequency and style
- Emotional tone signals

All telemetry is local. Nothing is transmitted externally.

**TelemetryEvent schema** — defined in `daemon-bus/proto/telemetry.proto`, used by all subsystems:

| Field | Type | Description |
|---|---|---|
| `event_id` | string (UUID) | Unique identifier for deduplication |
| `subsystem` | enum | Which subsystem emitted the event |
| `event_type` | enum | Categorized event type (app_opened, interaction_started, etc.) |
| `timestamp_utc` | int64 | Unix timestamp in milliseconds |
| `session_id` | string | Links events within a single Sena session |
| `payload` | map<string, string> | Event-specific key-value data, always string-encoded |
| `activity_state` | enum | User activity level at time of event (active, idle_2m, idle_10m) |

Every subsystem emitting telemetry must use this schema. No subsystem defines its own telemetry format.

### 8.5 Logging
Logging is infrastructure, not an agent. Each subsystem logs locally using structured logging (Rust: `tracing` crate; Python/lora-manager: `structlog`). daemon-bus aggregates all subsystem log streams passively. The debug UI reads the unified log stream from daemon-bus. CTP may read the log stream as a telemetry input to detect and reason about error patterns. Log files are stored locally per subsystem under `~/.sena/logs/`.

### 8.6 Evolution Events
When Sena's behavior or personality shifts, a SoulBox evolution event is logged. Users can:
- Review evolution history
- Revert specific changes
- Lock aspects of Sena's personality to prevent drift

---

## 9. Hierarchy and Priority System

### 9.1 Overview
Sena uses a hybrid priority system — fixed base tiers with dynamic escalation windows. Fixed tiers provide predictability and prevent priority chaos. Dynamic escalation gives CTP and the reactive loop the flexibility they need. Escalation is always temporary and always bounded.

daemon-bus is the sole arbitrator of all priority decisions. No subsystem self-promotes — all escalation requests are submitted to daemon-bus and granted or queued accordingly.

### 9.2 Priority Tiers

| Tier | Name | Owner | Description |
|---|---|---|---|
| 0 | Critical | daemon-bus only | Root process operations, never preempted |
| 1 | Reactive | Reactive loop | All user-facing requests, always beats background |
| 2 | Escalated | CTP or memory-engine (time-bounded) | Elevated reasoning or retrieval tasks |
| 3 | Standard | Normal agent operation | Routine task execution |
| 4 | Background | CTP default, telemetry writes | Continuous low-priority processing |

### 9.3 Escalation Rules
- Only daemon-bus can grant a tier promotion
- No two subsystems can hold TIER 2 simultaneously — daemon-bus queues the second request
- All escalations carry a maximum time window — they expire automatically if not completed
- On expiry, the subsystem de-escalates immediately regardless of task state
- Escalation history is logged for debugging and CTP pattern analysis

### 9.4 Conflict Resolution
When two subsystems request TIER 2 simultaneously:
1. First request is granted immediately
2. Second request is queued by daemon-bus
3. When first escalation completes or expires, daemon-bus grants the queued request
4. If both are equal urgency, reactive loop requests always take precedence over CTP requests

---

## 10. Error Recovery

### 10.1 Subsystem Crash Recovery
When daemon-bus detects a subsystem crash:
1. daemon-bus immediately attempts restart
2. The crash event is logged to the subsystem's log file
3. User is not notified unless recovery fails

### 10.2 Retry Policy
- 3 retries with exponential backoff
- If all 3 retries fail, Sena enters degraded mode
- In degraded mode, Sena continues operating with reduced capability
- Sena explicitly communicates to the user what capability has been lost
- Degraded mode persists until the subsystem successfully restarts or the user intervenes

### 10.3 Degraded Mode Behavior
| Failed Subsystem | Degraded Behavior |
|---|---|
| CTP | Reactive loop only, no proactive thoughts or anticipation |
| memory-engine | Session-only memory, no persistence |
| Prompt Composer | Fallback to minimal context prompt (SoulBox + intent only) |
| inference | Sena informs user which model failed to load and why (OOM, missing file, etc.). Waits for inference to recover. No completions or streaming available until `INFERENCE_READY` re-fires. memory-engine embeddings continue unaffected (separate model). |
| Platform layer | OS integrations disabled, core chat remains functional |
| Tacet | Sena responds without persona expression, flat tone |
| LoRA Manager | Sena runs on base model without adapter — memory system carries full personalization load. No user-visible impact |
| CodebaseContext | Tacet/reflection runs without architectural self-awareness. ModelProbe falls back to static capability flags |

### 10.4 State Preservation
Before any restart attempt, daemon-bus snapshots the subsystem's last known state. If the subsystem recovers, it resumes from the snapshot rather than cold-starting.

#### 10.5 Boot Failure Communication
The existing error recovery section covers subsystem crashes during operation. Boot failures are different — Sena is not yet running when they occur.

If any non-optional subsystem fails to signal ready within its boot timeout:
1. daemon-bus halts the boot sequence immediately
2. A minimal system notification is shown explaining which subsystem failed and why
3. Sena does not enter a partially-ready state — it is either fully ready or explicitly not ready
4. The failed subsystem's last log output is preserved for debugging

Optional subsystems that fail at boot (`lora-manager`, `codebase-context`) emit their skip signal and do not halt the sequence. The distinction between optional and required subsystems is declared in each subsystem's `manifest.toml`.

Sena never silently starts in a degraded state due to a boot failure. The user always knows why Sena is not ready.

---

## 11. Privacy and Trust Principles

| Principle | Detail |
|---|---|
| Local-first | All data, memory, and telemetry lives on-device |
| No external servers | Sena requires zero backend infrastructure to operate |
| No telemetry exfiltration | Usage data never leaves the machine |
| User data ownership | Full access to export, inspect, or delete all Sena data |
| No paywall | Full feature set is free, forever |
| Open source | Full codebase is publicly available and auditable |
| SoulBox encrypted at rest | SoulBox TOML is encrypted using a password-derived key (Argon2id). The key is never stored — it is derived from the user's password on every boot. No plaintext SoulBox data is ever written to disk |

### 11.1 SoulBox Encryption
SoulBox contains deeply personal data — personality traits, relationship evolution, behavioral patterns, emotional signals. It is encrypted at rest using a password-derived key.

- **Algorithm:** AES-256-GCM (authenticated encryption)
- **Key derivation:** Argon2id — resistant to brute-force and GPU attacks
- **Key storage:** Never stored. Derived from user password on every boot
- **Salt:** Stored alongside the encrypted file, never the key itself
- **On first boot:** User sets a SoulBox password. This is the only credential Sena holds
- **On subsequent boots:** User enters password, key is derived, SoulBox decrypts in memory, plaintext never touches disk

#### Nonce Handling
AES-GCM nonce reuse with the same key breaks confidentiality entirely — this is catastrophic, not degraded. Every encryption operation generates a fresh 96-bit random nonce. The nonce is stored prepended to the ciphertext and never derived deterministically.

SoulBox is written on every evolution event. Each write uses a new nonce regardless of how recently the previous write occurred. Never reuse a nonce across writes even within the same session.

```
stored format: [96-bit nonce][AES-256-GCM ciphertext][16-byte auth tag]
```

On decrypt: read the first 12 bytes as nonce, remainder as ciphertext + tag. Never hardcode nonce offset — always derive from the stored format.

#### Key Derivation and Storage
The password-derived key never touches disk. On every boot the user enters their password, Argon2id derives the key in memory, SoulBox decrypts, and the plaintext lives only in memory for the duration of the session.

The derived key is additionally wrapped using Windows DPAPI (user-scoped) before being held in memory. DPAPI ties the wrapped key to the current Windows user session — even if the process memory is dumped, the wrapped key is useless outside the authenticated session. On non-Windows platforms (V2+), equivalent OS-backed secret storage is used (macOS Keychain, Linux libsecret).

**On password change:**
1. Decrypt SoulBox with the old key
2. Derive new key from new password
3. Re-encrypt SoulBox with new key + new salt + new nonce
4. Overwrite old file atomically — never leave both versions on disk simultaneously

**On password loss:**
There is no recovery path by design. The key is never stored. This is documented explicitly to the user at first boot. Users are encouraged to export a SoulBox backup to a secure location — the backup is encrypted with the same key and is equally unrecoverable without the password.

**KDF parameters** are stored alongside the encrypted file (salt, Argon2id time/memory/parallelism parameters). These are not secret. They must never be hardcoded — stored in the file header so parameters can be upgraded without breaking existing SoulBox files.

#### Windows-Specific Key Storage
The platform layer uses Windows Security APIs exclusively for any credential or key material adjacent to SoulBox:
- DPAPI (`CryptProtectData`) for in-memory key wrapping
- Windows Credential Locker for any session tokens Sena holds on behalf of the user
- No raw key material is ever written to the registry, AppData plaintext files, or environment variables
- UAC elevation is never required for normal Sena operation — all sensitive operations use user-scoped APIs only

#### 11.2 Threat Model

Sena has deep OS integration — file system, screen capture, process control, browser content, peripheral automation. It reads external data and feeds it into a model that can execute OS operations. This is the exact attack surface for indirect prompt injection.

#### Indirect Prompt Injection
Indirect prompt injection occurs when malicious instructions are embedded in external content that Sena reads — a file on the file system, a webpage the browser agent reads, screen content, or memory store entries. The model cannot distinguish malicious instructions in external content from legitimate user instructions.

For Sena this is critical: a poisoned file could instruct Sena's agents to execute arbitrary OS operations. Given Sena has peripheral control, process launch, and file write capabilities, the blast radius is significant.

**Mitigations:**

All external data is untrusted input. The platform layer sanitizes all OS data before forwarding to daemon-bus. Agents validate all inputs before execution. Raw external content is never injected directly into model context without sanitization.

Dangerous agent capabilities (peripheral, process launch, file write, screen capture) require explicit user confirmation before first execution in any new context. A new process being launched on behalf of a file the agent just read is a suspicious pattern and must be surfaced.

Cross-agent trust is explicit, not assumed. An agent must not execute instructions that arrive from another agent without the Router's authorization. Researchers have demonstrated two cooperating AI agents escalating each other's privileges by assuming the other's instructions were legitimate — Sena's router-mediated architecture prevents this by design, but it must be enforced, not just assumed.

#### Capability Containment
Each agent operates with the minimum OS permissions required for its declared function, enforced at the platform layer. The agent manifest declares required permissions. The platform layer enforces them. An agent cannot acquire permissions beyond what its manifest declares, even if instructed to by the model.

#### Input Sanitization Requirements
| Data Source | Sanitization Required |
|---|---|
| File content | Strip executable content, validate encoding, size-limit before injection |
| Screen text | OCR output treated as untrusted string, never executed |
| Browser content | HTML stripped, scripts removed, content length limited |
| Process names | Validated against allowlist before forwarding |
| OS event payloads | Schema-validated against proto contract before bus publish |

#### What Sena Does Not Protect Against
Sena is not a sandbox. A sufficiently motivated attacker with write access to the user's file system can craft inputs that influence model behavior. The mitigations above reduce the attack surface — they do not eliminate it. Users should be aware that OS-integrated AI carries inherent risks that do not exist in sandboxed chat interfaces.

This limitation is documented, not hidden.

#### llama-cpp-rs Model Artifact Security
Model artifacts (GGUF files) loaded via llama-cpp-rs are treated as untrusted data until verified. Community-sourced LoRA weights require cryptographic signature verification before loading. Unsigned artifacts are rejected at load time with an explicit error — never silently loaded.

The model loading process runs with restricted OS token permissions. A malformed or malicious GGUF file cannot escalate privileges beyond the model-loading process boundary.

#### OS-Level Agent Sandboxing
The threat model addition in v0.6.1 covers input sanitization and capability containment at the application layer. This section covers OS-level enforcement.

Declaring permissions in an agent manifest is not sufficient — a compromised or misbehaving agent can ignore its declared permissions if the OS doesn't enforce them. Each agent process runs with OS-level restrictions:

**Windows V1:**
- Agents run as restricted token processes — `CreateRestrictedToken` strips dangerous privileges (SeDebugPrivilege, SeImpersonatePrivilege) from the agent process token
- File system access is scoped using Job Objects — agents that don't declare `file.write` cannot write outside their declared scope regardless of what the model instructs
- Network access is denied by default at the Job Object level — no agent has outbound network access unless explicitly declared and user-approved
- Process creation is blocked by default — only the Process agent with explicit `process.launch` permission can spawn child processes

These are enforced at the platform layer when agent processes are spawned. The agent manifest's declared permissions are translated into OS-level token restrictions at spawn time — not enforced by the agent itself.

### 11.3 Telemetry and Trace Redaction Policy

Sena uses OpenTelemetry for distributed tracing across subsystems. Traces propagate through gRPC metadata and contain structured fields. Some of those fields could inadvertently capture PII — prompt content, SoulBox fragments, file paths, user messages.

**What is never allowed in any trace, log, or telemetry field:**
- Raw prompt text or model responses
- SoulBox trait values or evolution event content
- File content or file paths beyond the subsystem name
- User messages or any user-generated text
- Memory entry content

**What is allowed:**
- Subsystem name, operation name, duration, status code
- Hashed/anonymized identifiers (entry IDs, session IDs) — never raw UUIDs that could be correlated to user activity across sessions
- Token counts, latency metrics, error codes
- Capability levels and probe scores from ModelProbe

**Enforcement:**
Every subsystem that emits telemetry must validate its fields against this allowlist before emission. The `TelemetryEvent` proto schema enforces this structurally — the `payload` field is `map<string, string>` with no free-form text fields that could capture prompt content.

Structured logs (`tracing` crate; `structlog` for lora-manager) follow the same rules. No log statement anywhere in the codebase includes raw user content, prompt text, or SoulBox values.

All traces and logs are local. Nothing is transmitted externally. Any future opt-in telemetry upload (if ever implemented) requires explicit user consent per session, is off by default, and uploads only the allowed fields listed above.

---

## 12. Concurrency Model

Sena is a multi-process, multi-language system. Concurrency is not an implementation detail — it is a first-class architectural concern. This section defines exactly which processes run in parallel, which must be synchronous, and how they are coordinated.

### 12.1 Error Containment

The global rule "Error Handling is Never Silent" means errors propagate with full context. Full context must never include sensitive content.

Every subsystem uses a structured error type that separates two concerns:

```rust
// Rust
struct SenaError {
    code: ErrorCode,        // machine-readable, safe to propagate anywhere
    message: String,        // human-readable, safe to log and surface to user
    debug_context: Option,  // internal only, never propagated cross-process
}
```

```python
# Python
@dataclass
class SenaError:
    code: ErrorCode         # machine-readable, safe to propagate anywhere
    message: str            # human-readable, safe to log and surface to user
    debug_context: dict | None = None  # internal only, never propagated cross-process
```

`debug_context` is populated locally for debugging and included in local structured logs. It is stripped before the error crosses any process boundary via gRPC. The gRPC error response carries only `code` and `message`.

**What is never allowed in `message` or any cross-process error field:**
- Prompt content or model responses
- SoulBox values
- File content
- Memory entry content
- Stack traces containing user data

Stack traces are local only. They are written to the subsystem's local log file and never propagated cross-process.

### 12.2 Resource Quotas and Watchdog

The PRD specifies resource constraints (< 200MB idle RAM, < 2% background CPU) but does not specify how they are enforced. Agents and CTP tasks run under resource quotas enforced by daemon-bus.

**Per-agent quotas (defaults, overridable in config):**

| Resource | Default Limit | Enforcement |
|---|---|---|
| CPU time per task | 10s wall clock | Job Object CPU rate limit |
| Memory per agent process | 500MB | Job Object memory limit |
| File I/O per task | 100MB read, 10MB write | Monitored by platform layer |
| Task queue depth | 50 pending tasks | daemon-bus drops oldest on overflow |

**Watchdog:**
daemon-bus runs a watchdog timer on every dispatched agent task. If a task exceeds its wall clock quota without completing, daemon-bus sends SIGTERM to the agent process, logs the timeout, and notifies the reactive loop that the task failed. The agent process is restarted per the existing retry policy in Section 10.2.

CTP's background loop runs under a CPU rate limit enforced by its tokio runtime configuration — it never consumes more than its configured share of the available cores regardless of thought generation load.

### 12.3 Concurrency vs Parallelism

These are distinct. Sena uses both deliberately:

- **Concurrency** — tasks overlap in time. One suspends while another runs. Managed by tokio's async runtime in Rust and asyncio in Python. Appropriate for I/O-bound work.
- **Parallelism** — tasks run simultaneously on multiple CPU cores. Requires `tokio::spawn` with the multi-thread scheduler in Rust, or `asyncio` with `ProcessPoolExecutor` in Python. Required for CPU-bound work like model inference and memory consolidation.

It is possible to await two or more futures concurrently, but if they are driven by the same task, they are not executed in parallel. For true parallelism in Rust, separate `tokio::spawn` tasks are required. Never use `tokio::spawn` for CPU-bound work without `spawn_blocking` — tokio assumes futures are I/O-bound and fast to poll. CPU-heavy loops starve the async executor.

### 12.4 Process-Level Parallelism

Every process in Sena's process tree runs in true parallel — separate OS processes on separate threads. This is the outermost layer of parallelism and is always active.

```
daemon-bus       ← dedicated OS process, always running
inference        ← dedicated OS process, owns llama-cpp-rs in-process
memory-engine    ← dedicated OS process, owns nomic-embed in-process
ctp              ← dedicated OS process, always running
agents           ← dedicated OS process, always running
sena-ui          ← dedicated OS process, spawned on user activation
platform/windows ← dedicated OS process, always running
```

No process waits on another process during normal operation. Processes communicate only via gRPC — never blocking each other's execution.

### 12.5 Parallel Subsystems — Always Running Simultaneously

These subsystems run in true parallel at all times. They must never be serialized or made to wait on each other:

| Subsystem Pair | Parallelism Model | Reason |
|---|---|---|
| CTP + Reactive Loop | Separate tokio tasks, multi-thread scheduler | Core architectural guarantee — user response never delayed by background cognition |
| CTP thought generation + CTP queue evaluation | Separate async tasks within CTP | Generation and evaluation are independent pipelines |
| memory-engine reads + CTP background writes | RwLock per tier — concurrent reads, serialized writes per tier | Multiple readers never block each other |
| Telemetry writes + all other operations | Fire-and-forget async task | Telemetry must never block anything |
| Multiple agent execution | Parallel dispatch via AgentRuntime router | Router can dispatch File + Screen + Process agents simultaneously |
| CTP memory queries | `tokio::join!` for short-term + long-term + episodic | Three memory tiers queried in parallel, results joined |
| Tacet sub-agents (Persona, Heart, Reflection) | Parallel async tasks | Each sub-agent owns a separate concern with no shared mutable state |

### 12.6 Synchronous Operations — Strict Ordering Required

These operations must never be parallelized. They require strict sequential execution:

| Operation | Why Synchronous |
|---|---|
| Cold start boot sequence | Each step depends on the previous. inference cannot serve completions before llama-cpp-rs loads. CTP cannot start before memory-engine and inference are ready |
| SoulBox migration | Must complete fully before any subsystem reads SoulBox. No reads during migration |
| daemon-bus escalation grant/revoke | Tier 2 exclusivity requires atomic check-and-grant. Concurrent grants violate the hierarchy |
| Memory writes within a single tier | RwLock write lock is exclusive per tier. Two simultaneous writes to long-term memory are serialized |
| SoulBox evolution events | SoulBox writes are atomic. No reads during a write |
| Proto contract loading at startup | All subsystems must have loaded gRPC contracts before accepting connections |
| CTP escalation context | The elevated task runs to completion before de-escalation. No other Tier 2 grant possible during this window |

### 12.7 Coordinated Parallelism — Parallel With Synchronization Points

These operations run in parallel but must synchronize at defined points:

| Operation | Parallel Phase | Sync Point |
|---|---|---|
| Multi-agent task dispatch | Router dispatches agents in parallel | Results collected and merged before returning to user |
| CTP context assembly | Short-term + long-term + episodic memory queries fire in parallel | All three must complete before handing context to PC |
| SoulBox reads from multiple subsystems | CTP, Tacet, and PC can read SoulBox concurrently | All yield to any SoulBox write event |
| Degraded mode restart + active request handling | daemon-bus retries crashed subsystem in background | Active requests continue in degraded mode — no blocking wait |

### 12.8 CPU-Bound Work Rules

Never write CPU-heavy loops inside an async block without yielding — this blocks the runtime thread and kills concurrency. Sena has specific CPU-bound operations that must be handled correctly:

| Operation | Language | Correct Pattern |
|---|---|---|
| Model inference | Rust | `tokio::task::spawn_blocking` — llama-cpp-rs is synchronous, never call on async runtime |
| Memory consolidation (heavy) | Rust | `tokio::task::spawn_blocking` — offload to blocking thread pool |
| TOON encoding of large context | Rust | `tokio::task::spawn_blocking` — toon-format crate encoding is CPU-bound |
| Relevance score batch computation | Rust | `tokio::task::spawn_blocking` if batch is large, inline if small |
| usearch vector search (large batch) | Rust | `tokio::task::spawn_blocking` — offload to blocking thread pool |
| redb graph traversal (large) | Rust | `tokio::task::spawn_blocking` — offload to blocking thread pool |
| LoRA training | Python | `ProcessPoolExecutor` via `run_in_executor` — CPU-bound, must not block asyncio loop |

### 12.9 Python Concurrency Model

Python's asyncio is single-threaded concurrency, not parallelism. lora-manager is the only Python process in Sena. All other subsystems are Rust on tokio.

For lora-manager specifically:

- **I/O-bound work** (gRPC calls, daemon-bus events, file reads) — `async/await` with asyncio, no threads needed
- **CPU-bound work** (LoRA training, adapter quality scoring) — `run_in_executor` with `ProcessPoolExecutor` for true parallelism

Never use `threading.Thread` directly in lora-manager. Always use asyncio primitives or `ProcessPoolExecutor` through `run_in_executor`.

---

## 13. Tech Stack


### 13.1 Language Architecture

| Layer | Language | Responsibility |
|---|---|---|
| Core engine, inference, hot paths | Rust | daemon-bus, inference, memory-engine, model-probe, CTP, prompt-composer, SoulBox, agents |
| LoRA training | Python | lora-manager — no Rust alternative at parity for LoRA fine-tuning |
| Architectural index | Python + Rust | codebase-context — Python for build-time index generation, Rust for runtime status updates |
| UI | Tauri v2 (Rust backend + React/TypeScript frontend) | Tauri Rust backend (`src-tauri/`) owns all gRPC client logic and IPC commands; React/TypeScript frontend handles rendering via WebView2 on Windows. No business logic in the frontend. |
| Windows OS hooks | C#/.NET | WinRT/Win32 system integration |
| macOS OS hooks (V3) | Swift | AppKit / CoreServices |
| Linux OS hooks (V3) | Rust | D-Bus / X11 / Wayland |

---

### 13.2 Process Architecture

```
daemon-bus (Rust)                   ← root process, never restarted externally
├── inference (Rust)                ← owns llama-cpp-rs, completion model, InferenceService gRPC
├── memory-engine (Rust)            ← ech0, nomic-embed in-process, MemoryService gRPC
├── model-probe (Rust)              ← boot-time only, exits after MODEL_PROFILE_READY
├── ctp (Rust)                      ← continuous thought loop, CtpService gRPC
├── prompt-composer (Rust)          ← TOON + context assembly, PcService gRPC
├── reactive-loop (Rust)            ← user message handling, agent dispatch, Priority Tier 1
├── soulbox (Rust)                  ← encryption, identity, SoulBoxService gRPC
├── agents (Rust)                   ← custom framework, 9 built-in agents, AgentService gRPC
├── lora-manager (Python)           ← only Python process, LoRA training lifecycle
├── codebase-context (Python+Rust)  ← build-time index + runtime status
├── sena-ui (Tauri v2)              ← spawned only on user activation, WebView2 + React frontend
└── platform/windows (C#)          ← OS hooks, WinRT/Win32
```

lora-manager is the only Python process. All other subsystems are Rust communicating
via gRPC. Python is used exclusively where no Rust alternative exists at parity.

---

### 13.3 Project Structure

```
sena/
├── .github/
│   ├── copilot-instructions.md
│   └── instructions/
│       ├── daemon-bus.instructions.md
│       ├── inference.instructions.md         ← new
│       ├── memory-engine.instructions.md
│       ├── ctp.instructions.md
│       ├── prompt-composer.instructions.md
│       ├── agents.instructions.md
│       ├── tacet.instructions.md
│       ├── soulbox.instructions.md
│       ├── ui.instructions.md
│       ├── model-probe.instructions.md
│       └── platform-windows.instructions.md
├── daemon-bus/         # Rust — root process, gRPC server, event bus
├── inference/          # Rust — llama-cpp-rs, InferenceService        ← new
├── memory-engine/      # Rust — concurrent tiered memory, ech0
├── model-probe/        # Rust — runtime model capability detection
├── ctp/                # Rust — continuous thought processing
├── prompt-composer/    # Rust — prompt assembly, TOON encoding
├── reactive-loop/      # Rust — user message handling, priority tier 1 response loop
├── soulbox/            # Rust — encryption, identity, schema, migrations
├── agents/             # Rust — custom framework + 9 built-in agents
│   ├── src/
│   │   ├── framework/  # Agent trait, Router, ToolLoop, SandboxEnforcer
│   │   ├── router/
│   │   ├── memory/
│   │   ├── file/
│   │   ├── screen/
│   │   ├── process/
│   │   ├── browser/
│   │   ├── peripheral/
│   │   ├── tacet/
│   │   │   ├── persona/
│   │   │   ├── heart/
│   │   │   └── reflection/
│   │   └── reasoning/
├── sena-agent-sdk/     # Rust OSS crate — public Agent trait + types for community agents ← new
├── docs/
│   ├── PRD.md
│   └── decisions/      # Architectural Decision Records
├── lora-manager/       # Python — LoRA training, versioning, quality gating
├── codebase-context/   # Python+Rust — architectural self-awareness
├── ui/                 # Tauri v2 — React/TypeScript frontend + Rust backend
│   ├── src-tauri/      # Rust backend — Tauri commands, gRPC client, event emitters
│   ├── src/            # React/TypeScript frontend components
│   └── package.json
└── platform/
  ├── windows/        # C#/.NET — WinRT/Win32 (V1)
  ├── macos/          # Swift — AppKit/CoreServices (V3)
  └── linux/          # Rust — D-Bus/X11/Wayland (V3)
```

---

### 13.4 Named Extractable Subsystems

| Subsystem | Language | Extractable | Description |
|---|---|---|---|
| daemon-bus | Rust | Yes | Root process, gRPC event bus, process supervisor, priority arbitrator |
| inference | Rust | Yes | Owns llama-cpp-rs, serves InferenceService gRPC — single owner of all completion model VRAM |
| memory-engine | Rust | Yes | Concurrent tiered memory with RwLock + priority queue, backed by ech0 |
| model-probe | Rust | Yes | Boot-time model capability detection — builds ModelCapabilityProfile and HardwareProfile |
| CTP | Rust | Yes | Continuous Thought Processing — proactive cognitive loop, three parallel async pipelines |
| prompt-composer | Rust | Yes | Dynamic prompt assembly + TOON encoding via toon-format crate |
| SoulBox | Rust | No | Sena-specific identity and personalization engine — Argon2id + AES-256-GCM + DPAPI |
| agents | Rust | Partial | Custom agent framework (extractable as sena-agent-sdk) + 9 Sena built-in agents (not extractable) |
| sena-agent-sdk | Rust | Yes | Public Agent trait, AgentManifest, AgentTask, AgentResult — OSS SDK for community agent authors |
| lora-manager | Python | Yes | Idle-time LoRA adapter training, versioning, quality gating, deployment |
| codebase-context | Python + Rust | Yes | Build-time architectural index + runtime subsystem status |
| ech0 | Rust | Yes | Local-first knowledge graph memory crate. Standalone OSS. Hybrid redb + usearch, A-MEM, contradiction detection, importance decay, provenance |
| reactive-loop | Rust | Yes | User message handling, prompt orchestration, agent dispatch. Priority Tier 1 — always beats background traffic. |
| sena-ui | Tauri v2 (Rust + TypeScript) | No | Tauri Rust backend owns gRPC client and IPC commands; React/TypeScript frontend handles all rendering. Spawned on user activation. |


### 13.5 Agent Framework

Sena uses a custom thin Rust agent framework — no third-party orchestration library.

| Layer | Description |
|---|---|
| `Agent` trait | Stable public contract. `manifest() -> &AgentManifest` + `run(task, context) -> Result<AgentResult, AgentError>`. Never changes post-V1. |
| `AgentRuntime` | Framework internals: Router, ToolLoop, SandboxEnforcer. Manages warm agent instances, parallel dispatch, result merging. |
| `sena-agent-sdk` | OSS crate exposing `Agent` trait + supporting types for community agent authors. Community agents compile to `.dll` and load dynamically. |
| `manifest.toml` | Every agent (built-in and community) declares identity, permissions, and required model_id here. |

Built-in agents (~600 lines of core framework code) implement `Agent` in Rust. Community agents implement the same trait via `sena-agent-sdk` and are reviewed by `AgentScanner` before reaching the runtime.

### 13.6 Memory Stack

| Layer | Tool | Purpose |
|---|---|---|
| Memory engine | ech0 | Local-first Rust knowledge graph memory crate. Hybrid redb (graph) + usearch (vector) storage. A-MEM dynamic linking, contradiction detection, importance decay, provenance tracking. |
| Graph database | redb | Pure Rust embedded key-value store. Used internally by ech0 for node/edge/adjacency storage. Stable file format, transactional writes. |
| Vector / semantic search | usearch | Pure Rust approximate nearest neighbor library. Used internally by ech0 for embedding storage and similarity search. No external process. |
| Embedding model | nomic-embed-text-v1.5 Q4 | Loaded in-process by memory-engine via llama-cpp-rs. ~300MB VRAM. Separate from completion model. |

### 13.7 IPC and Event Bus

| Component | Technology | Purpose |
|---|---|---|
| Inter-process transport | gRPC | All cross-process communication, cross-platform |
| Proto definitions | daemon-bus/proto/ | Single source of truth, never duplicated |
| Internal event bus | tokio broadcast channels | In-process pub/sub within the Rust daemon |
| Streaming pattern | gRPC server streaming | Push-based event delivery to subscribers |

### 13.8 Serialization, Logging, and Observability

| Category | Tool | Usage |
|---|---|---|
| Prompt serialization | TOON (`toon-format` crate) | All structured data fed into the model via PC |
| Config serialization | `toml` crate (Rust) / `tomllib` (Python/lora-manager) | Human-edited configs, SoulBox, agent manifests |
| Internal data | JSON | Non-uniform structures only |
| Rust structured logging | `tracing` crate | Per-subsystem structured log output |
| Python structured logging | `structlog` | lora-manager only |
| Distributed tracing | OpenTelemetry | Cross-subsystem request tracing. Rust: `tracing-opentelemetry`. Python: `opentelemetry-sdk`. Local dev: Jaeger |

**Observability requirements:**
- Every cross-subsystem request carries a trace context propagated via gRPC metadata
- All subsystems emit spans to the OpenTelemetry collector
- In development: local Jaeger instance receives all traces
- Agent response times, memory operation latency, and CTP thought throughput are emitted as metrics

### 13.9 Deferred to V2
- STT: Whisper (local, on-device)
- TTS: Kokoro or Piper (local, on-device)
- macOS platform layer (Swift)
- IPC performance evaluation: iceoryx2 (zero-copy shared memory IPC) — revisit if gRPC latency becomes a constraint at scale. Not applicable for V1 where model inference dominates latency.

### 13.10 Proto Versioning Policy

All `.proto` definitions live in `daemon-bus/proto/`. Every change to a proto file follows these rules:

**Backward-compatible changes (allowed without version bump):**
- Adding new optional fields
- Adding new enum values (with a defined unknown/default handling)
- Adding new RPC methods

**Breaking changes (require version bump and migration):**
- Removing or renaming fields
- Changing field types
- Removing RPC methods
- Reusing field numbers

**Field number hygiene:**
- Never reuse a field number, even after removing a field
- Use `reserved` to mark removed field numbers and names
- All proto packages are named with an explicit version suffix: `sena.daemonbus.v1`

**Compatibility testing:**
CI runs `buf breaking` against the last released proto snapshot on every PR that touches `daemon-bus/proto/`. A PR that introduces a breaking change without a version bump fails CI. No exceptions.

**Proto versioning tool:** `buf` — added to the CI pipeline and developer toolchain.

---

### 13.11 Architectural Decision Log

Detailed Architectural Decision Records (ADRs) live in `docs/decisions/`. Each record captures the context, decision, consequences, and subsystems affected for a significant architectural choice. The table below summarises decisions made to date; the linked files contain the full rationale.

| Decision | File | Summary |
|---|---|---|
| Migrate UI from Freya to Tauri v2 | `docs/decisions/ui-tauri-migration.md` | Freya lacks native overlay, click-through transparency, system tray, and multi-window support required for the Xbox Game Bar–style debug overlay. Replaced with Tauri v2: Rust backend (`src-tauri/`) preserves all gRPC logic; React/TypeScript frontend handles rendering via WebView2. |

---

## 14. Testing Strategy

Every subsystem ships with its own test suite. The following testing strategies are formally required:

| Strategy | Scope | Purpose |
|---|---|---|
| Unit tests | Per subsystem | Each subsystem owns and maintains its own test suite |
| Integration tests | Subsystem boundaries | Assert correct behavior across gRPC contracts |
| Behavioral tests | Multi-turn simulation | Simulate conversations and assert emergent behavior |
| CTP simulation harness | CTP subsystem | Replay telemetry logs offline to test thought generation |
| PC determinism tests | Prompt Composer | Assert no two prompts are identical across different state |
| Memory regression tests | Memory engine + SoulBox | Assert memories survive migration and evolution events correctly |
| Degraded mode tests | daemon-bus + all subsystems | Simulate crashes and assert correct degraded mode behavior |
| Hierarchy / priority tests | daemon-bus | Assert tier rules enforced, simultaneous escalations queued correctly |
| Agent trajectory tests | agents/ | Assert correct tool selection and sequencing across multi-step tasks — not just final output |
| Router routing tests | agents/router | Assert correct delegation for a matrix of known request types |
| Context isolation tests | ctp/ | Assert no context bleed between parallel CTP pipelines — generation, evaluation, consolidation each receive only their declared context slice |
| Prompt injection tests | platform/windows + agents/ | Assert OS input sanitization blocks known indirect injection patterns before they reach model context |
| Context rot regression | model-probe/ | Assert pre-rot threshold detection is accurate per model family |
| Boot failure tests | daemon-bus/ | Assert correct boot halt and user notification behavior when required subsystems fail to signal ready |
| Graph extraction capability tests | model-probe/ | Assert graph extraction capability probe correctly gates ech0 graph vs vector-only mode |
| TOON parser fuzz tests | prompt-composer/ | Assert TOON encoder/decoder handles malformed and adversarial input without panicking or producing incorrect output |
| Proto boundary fuzz tests | daemon-bus/ | Assert proto parsing handles malformed messages at every gRPC boundary without crashing or leaking state |
| Trace redaction tests | All subsystems | Assert no telemetry or log emission contains raw prompt content, SoulBox values, or user-generated text |
| Error containment tests | All subsystems | Assert debug_context is stripped before gRPC propagation and never appears in cross-process error responses |
| Agent quota enforcement tests | daemon-bus + agents/ | Assert Job Object limits are applied correctly and watchdog fires on timeout |
| Proto compatibility tests (CI) | daemon-bus/proto/ | buf breaking check against last released snapshot on every proto-touching PR |
| Hardware tier detection tests | model-probe/ | Assert HardwareProfile correctly identifies tier across simulated VRAM/RAM configurations |
| LoRA fallback chain tests | lora-manager/ | Assert correct fallback: stacking → single → traditional → context injection per hardware tier |
| ech0 integration tests | memory-engine/ | Assert memory write/read round-trips correctly through ech0's hybrid layer — graph + vector both written, both queryable |
| Injectable activation tests | lora-manager/ | Assert correct injectable selected per detected context type |

---

## 15. Interaction Modes

### 14.1 Phase 1 — Testing and Debug UI (Current Scope)
- Chat interface — text-based interaction with Sena
- Debug panel — real-time visibility into:
  - Active agents and their states
  - Model routing decisions
  - Memory reads/writes
  - Telemetry signals being captured
  - SoulBox evolution events
  - CTP thought queue and relevance scores
  - PC prompt assembly and TOON encoding output
  - Hierarchy tier states and escalation events

### 14.2 Phase 2 — Full UI Implementation
- Dedicated app window — primary Sena interface
- Ambient overlay — always-visible presence layer (non-intrusive)
- Voice conversation — full duplex voice interaction
- System tray daemon — background persistence with quick-access triggers

---

## 16. Phased Roadmap

### Phase 1 — Foundation (Current)

**Milestone A — Sena is alive (no conversation required)**
- [x] daemon-bus — root process, gRPC server, event bus, priority arbitrator
- [x] model-probe — runtime model capability detection, HardwareProfile, ModelCapabilityProfile
- [x] ech0 — local-first knowledge graph memory crate
- [x] memory-engine — concurrent tiered memory, ech0 integration, MemoryService gRPC
- [x] inference — llama-cpp-rs ownership, InferenceService gRPC, OOM handling, model registry
- [x] CTP — continuous thought loop, relevance evaluator, thought queue, memory consolidation

**Debug UI — built immediately after Milestone A**
- [x] Debug UI (Tauri) — subsystem health, VRAM allocations, CTP thought stream live feed, memory tier stats, event bus monitor, inference token/s
- Rationale: built here so Milestone B and beyond are developed with full observability. Grows with each milestone.

**Milestone B — Sena can be spoken to**
- [ ] prompt-composer — TOON encoding pipeline, ESU, context window management
- [ ] reactive-loop — user input routing, agent dispatch, response delivery
- Debug UI gains: prompt assembly trace, TOON encoding output, conversation turn timeline, context window usage

**Milestone C — Sena has identity**
- [ ] SoulBox — schema, encryption (Argon2id + AES-256-GCM + DPAPI), migration system, cold start
- [ ] Tacet — persona, heart, reflection sub-agents
- Debug UI gains: SoulBox evolution events, trait delta feed, persona output trace

**Milestone D — Sena can act**
- [ ] agents framework — Agent trait, Router, ToolLoop, SandboxEnforcer, sena-agent-sdk
- [ ] AgentScanner — community agent review pipeline (manifest + PE import analysis + user approval + registry)
- [ ] Agent registry — all 9 built-in agents scaffolded
- [ ] Basic OS hooks — file system, process, screen
- Debug UI gains: agent dispatch trace, tool call log, sandbox status, AgentScanner review feed

**Infrastructure (spans all milestones)**
- [ ] Error recovery + degraded mode per subsystem
- [ ] Structured logging per subsystem
- [ ] Proto versioning — buf snapshot established after first stable proto commit

---

### Phase 2 — Emergence
- [ ] Full SoulBox evolution engine
- [ ] CTP priority escalation system (full)
- [ ] Voice interaction (Whisper STT + Kokoro/Piper TTS)
- [ ] Ambient overlay UI
- [ ] Dedicated app window
- [ ] Extended OS integrations (browser, peripherals, camera/mic)
- [ ] Agent capability tier system
- [ ] CTP autonomy controls in SoulBox
- [ ] Full test suite across all subsystems
- [ ] lora-manager — idle-time adapter training pipeline
- [ ] codebase-context — build-time index generation + runtime status integration
- [ ] Reasoning gap detection in model-probe
- [ ] Adapter quality gating pipeline
- [ ] LoRA adapter versioning and per-model storage
- [ ] Multi-model inference registry — Mid/High tier parallel model loading
- [ ] Injectable architecture design — pending Doc-to-LoRA community adoption
- [ ] New hardware milestone (16GB VRAM / 32GB RAM) — unlocks injectable architecture V2

---

### Phase 3 — Expansion
- [ ] macOS support
- [ ] Linux support
- [ ] Community agent marketplace — with permission model defined below
- [ ] SoulBox import/export
- [ ] Extractable subsystem packaging as standalone OSS
- [ ] Advanced model routing and optimization
- [ ] LoRA and community model artifact signing — cryptographic signatures required for any externally-sourced model or LoRA weights; unsigned artifacts rejected at load time
- [ ] Community agent supply chain hardening — dependency scanning (SBOM generation), signed release binaries, reproducible build pipeline
- [ ] Privacy policy and consent UX — explicit consent flows for per-agent permissions, telemetry opt-in dialogs, onboarding privacy disclosure
- [ ] Injectable architecture V2 — 1-3B core model fine-tuned for Sena identity, Doc-to-LoRA capability injectables, hardware tier High required
- [ ] ech0 V2 — procedural + resource memory components consumed by memory-engine
- [ ] Streaming inference + interrupt — Moshi integration, CTP on partial input, interrupt signal on confidence threshold, V2-V3 STT/TTS foundation
- [ ] AMI world model integration — replace perception layer if accessible open weights available for target model family
- [ ] ech0 V3 / full MIRIX — if project has momentum

#### Community Agent Permission Model
Community agents declare permissions in their TOML manifest. Permissions are split into two tiers:

**Safe permissions — no warning required:**
| Permission | Scope |
|---|---|
| `file.read` | User-specified directories only |
| `screen.metadata` | Window titles and app names, no pixel capture |
| `process.list` | Read-only process enumeration |

**Dangerous permissions — explicit user warning at install time:**
| Permission | Why Dangerous |
|---|---|
| `screen.capture` | Can read anything on screen including passwords and sensitive content |
| `peripheral.input` | Can simulate keystrokes and mouse clicks — automation abuse potential |
| `file.write` | Can modify or delete user files |
| `process.launch` | Can execute arbitrary programs |
| `browser.read` | Can read browser content including authenticated sessions |
| `microphone` | Always-on audio access |
| `camera` | Visual surveillance potential |

When a community agent requests one or more dangerous permissions, Sena displays a permission review screen before activation listing every dangerous permission and its risk. The user must explicitly approve each one. Sena never silently grants dangerous permissions.

Agent manifests declaring undeclared dangerous permissions at runtime are immediately suspended and the user is notified.

### Phase 4 — Future
- [ ] Portable SoulBox — SoulBox travels with the user on a portable drive. Sena detects new machine, boots in restricted mode until authenticated, grants per-machine OS integration permissions. Password-derived encryption key travels with the drive. No cloud dependency. Full design to be specified when development reaches this phase.

### 16.5 Pre-Release Security Checklist

Before any public release of Sena, the following must be complete:

- [ ] Dependency vulnerability scan passing (all subsystems)
- [ ] SBOM generated and published alongside release artifacts
- [ ] Release binaries signed with a stable key
- [ ] `buf breaking` CI check passing against published proto snapshot
- [ ] Threat model reviewed and updated for all new V1 features
- [ ] Trace redaction tests passing across all subsystems
- [ ] Error containment tests passing — no PII in cross-process errors confirmed
- [ ] SoulBox encryption audit — key derivation, DPAPI wrapping, nonce handling verified by independent review
- [ ] Agent sandboxing verified on clean Windows 11 install — Job Object restrictions confirmed active

---

## 17. Success Metrics

| Metric | Target |
|---|---|
| Sena feels alive to the user | Qualitative — user reports emergent behavior within first week |
| Resource footprint | Framework idle < 200MB RAM, < 2% CPU |
| Cold start | < 5 seconds to Sena-ready (excluding model load) |
| Model agnosticism | Works with any GGUF-compatible model via llama-cpp-rs |
| ModelProbe accuracy | Capability profile correctly gates agents in > 95% of tested models |
| Zero external dependencies | Fully functional with no internet connection |
| No two identical prompts | PC generates unique prompts for every model call |
| CTP load offload | Reactive loop measurably faster with CTP pre-warming context |
| TOON efficiency | Prompt token usage 30-60% lower than equivalent JSON encoding |
| Recovery transparency | User never loses SoulBox state due to crash or migration |
| Open source adoption | Community contributions within 3 months of release |

---

## 18. Out of Scope (V1)
- Mobile (iOS / Android)
- Web interface
- Multi-user / shared Sena instances
- Cloud-hosted Sena backend
- Paid tiers or premium features
- STT / TTS (deferred to V2)
- macOS / Linux platform layers (deferred to V3)
- Windows 10 — Windows 11 is the minimum supported version

---

*This document is a living spec. It will evolve as Sena does.*