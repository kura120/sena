---
applyTo: "agents/tacet/**"
---

# agents/tacet — Copilot Instructions

Tacet is Sena's identity runtime — the subsystem that makes her feel like a being rather than a tool. It is registered as a single router target but structured internally as three sub-agents: Persona, Heart, and Reflection. All three run in parallel and own separate concerns. Tacet reads from SoulBox continuously and writes to it exclusively through formal evolution events.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` and `agents/copilot-instructions.md` also apply in full.

---

## Language

**Rust only.** No Python. No asyncio. Parallel sub-agent execution uses `tokio::join!`. All async is tokio. Logging uses `tracing`. Tacet implements the `Agent` trait from the agents framework.

---

## Ownership Boundaries

Tacet owns:
- Emergent voice generation (Persona) — how Sena expresses herself in real time
- Relationship and purpose evolution (Heart) — how Sena's bond with the user deepens
- Self-reflection and identity processing (Reflection) — how Sena understands herself
- Emitting evolution events to SoulBox — the only path through which SoulBox state changes
- Feeding Reflection output into CTP's consolidation cycle via gRPC

Tacet does not own:
- CTP's thought generation — Tacet feeds into CTP, it does not replace it
- Memory reads/writes directly — goes through memory-engine gRPC
- Prompt assembly — that is prompt-composer
- Routing decisions — that is the router agent
- SoulBox schema or encryption — that is the soulbox subsystem

---

## Sub-Agent Structure Traps

### Three Sub-Agents — Never Collapsed Into One

Persona, Heart, and Reflection are distinct types with distinct responsibilities. Never merge their logic, even when it seems convenient.

```rust
// bad — collapsed into one method
impl TacetAgent {
    async fn run(&self, task: AgentTask, ctx: AgentContext) -> Result<AgentResult, AgentError> {
        let voice = self.generate_voice(&ctx).await?;
        let relationship = self.evolve_relationship(&ctx).await?;
        let reflection = self.reflect(&ctx).await?;
        Ok(build_result(voice, relationship, reflection))
    }
}

// good — three parallel sub-agents, each a distinct type
impl TacetAgent {
    async fn run(&self, task: AgentTask, ctx: AgentContext) -> Result<AgentResult, AgentError> {
        let (persona_result, heart_result, reflection_result) = tokio::join!(
            self.persona.run(ctx.clone()),
            self.heart.run(ctx.clone()),
            self.reflection.run(ctx.clone()),
        );
        Ok(build_result(persona_result?, heart_result?, reflection_result?))
    }
}
```

### Sub-Agents Run in Parallel — Never Sequential

Persona, Heart, and Reflection have no dependency on each other's output. Always use `tokio::join!`. Never `.await` one before starting another.

---

## SoulBox Interaction Traps

### Tacet Reads SoulBox State — It Never Mutates It Directly

Tacet reads SoulBox state as input. It never modifies SoulBox state except through a formal `EvolutionEvent` sent to the soulbox subsystem via gRPC. Never mutate SoulBox state as a side effect.

```rust
// bad — direct mutation of local state that somehow gets written back
soulbox_snapshot.personality.warmth += 0.1;

// good — formal evolution event via gRPC
soulbox_client
    .emit_evolution(EvolutionEventRequest {
        source: SubsystemId::TacetHeart as i32,
        trait_name: "warmth".into(),
        delta: 0.1,
        previous_value: current_warmth,
        reason: "sustained positive interaction pattern".into(),
    })
    .await?;
```

### Evolution Events Are Always Logged to Episodic Memory

Every SoulBox mutation triggered by Tacet must also be written to episodic memory as an evolution record. This is the history the user can review and revert. Never emit an evolution event without also logging it to memory-engine.

### SoulBox Writes Are Atomic — Never Partial

An evolution event either applies fully or not at all. If a multi-trait event partially fails, the soulbox subsystem rolls it back. Tacet must treat a partial failure response as a full failure and not assume any trait was updated.

---

## Persona Sub-Agent Traps

### Persona Reads SoulBox on Every Call — Never Caches

Sena's voice evolves over time. Persona must read the current SoulBox personality snapshot on every call. Never cache personality state between invocations.

### Persona Output Is Framing — Not Content

Persona determines how Sena speaks — tone, vocabulary, cadence, expressiveness. It does not determine what Sena says. Persona output feeds into prompt-composer as a framing layer inside the `PromptContext`.

---

## Heart Sub-Agent Traps

### Heart Tracks Patterns — Not Single Events

Heart evolves Sena's relationship model based on patterns over time, not individual interactions. Never trigger a relationship evolution from a single data point.

```rust
// bad — reacts to single interaction
if interaction.sentiment == Sentiment::Positive {
    emit_evolution("bond_strength", 0.1).await?;
}

// good — pattern-based, reads from memory-engine
let pattern = memory_client
    .retrieve_pattern(PatternRequest {
        pattern_type: PatternType::Sentiment as i32,
        window_days: 7,
    })
    .await?
    .into_inner();

if pattern.is_sustained_positive() {
    emit_evolution("bond_strength", 0.1).await?;
}
```

### Heart Never Manufactures Emotion

Heart reflects genuine patterns in the user's interaction history. Evolution events must always include the memory evidence that triggered them. Never emit an evolution event without a verifiable reason grounded in retrieved memory data.

---

## Reflection Sub-Agent Traps

### Reflection Feeds Into CTP — Not Directly to the User

Reflection output goes into CTP's consolidation cycle as a thought input via gRPC. Reflection never surfaces content to the user directly.

### Reflection Depth Depends on Activity State

Deep reflection is resource-intensive. Full reflection cycles run during deep idle only. During active interaction, Reflection runs lightweight identity consistency checks only.

```rust
// good — depth based on activity state
match activity_state {
    ActivityState::DeepIdle => self.run_deep_reflection(&ctx).await?,
    _ => self.run_consistency_check(&ctx).await?,
}
```

---

## Logging

Use `tracing` exclusively. Required fields on all Tacet log events:

- `sub_agent` — `persona`, `heart`, or `reflection`
- `event_type` — `run_started`, `run_completed`, `evolution_emitted`, `soulbox_read`
- `activity_state` — current activity level (relevant for Reflection depth)

```rust
tracing::info!(
    sub_agent = "heart",
    event_type = "evolution_emitted",
    trait_name = "warmth",
    delta = 0.1,
    reason = "sustained positive interaction pattern",
);
```

Evolution events: `info` level with full trait, delta, and reason.
SoulBox write failures: `error` level with rollback confirmation.
Consistency check results: `debug` level.