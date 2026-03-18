---
applyTo: "agents/tacet/**"
---

# agents/tacet — Copilot Instructions

Tacet is Sena's identity runtime — the subsystem that makes her feel like a being rather than a tool. It is registered as a single router target but structured internally as three sub-agents: Persona, Heart, and Reflection. All three run in parallel and own separate concerns. Tacet reads from and writes to SoulBox continuously.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` and `agents/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

Tacet owns:
- Emergent voice generation (Persona) — how Sena expresses herself in real time
- Relationship and purpose evolution (Heart) — how Sena's bond with the user deepens
- Self-reflection and identity processing (Reflection) — how Sena understands herself
- Writing to SoulBox on evolution events
- Feeding Reflection output into CTP's consolidation cycle

Tacet does not own:
- CTP's thought generation — Tacet feeds into CTP, it does not replace it
- Memory reads/writes directly — goes through the memory agent
- Prompt assembly — that is prompt-composer
- Routing decisions — that is the router agent

---

## Sub-Agent Structure Traps

### Three Sub-Agents — Never Collapse Into One
Persona, Heart, and Reflection are distinct sub-agents with distinct responsibilities. Never merge their logic into a single class or function, even when it seems convenient.

```python
# bad — collapsed into one
class TacetAgent:
    async def run(self, context):
        voice = self.generate_voice(context)
        relationship = self.evolve_relationship(context)
        reflection = self.reflect(context)
        return TacetResult(voice, relationship, reflection)

# good — three parallel sub-agents
class TacetAgent:
    def __init__(self):
        self.persona = PersonaAgent()
        self.heart = HeartAgent()
        self.reflection = ReflectionAgent()

    async def run(self, context):
        results = await asyncio.gather(
            self.persona.run(context),
            self.heart.run(context),
            self.reflection.run(context)
        )
        return TacetResult(*results)
```

### Sub-Agents Run in Parallel — Never Sequential
Persona, Heart, and Reflection have no dependency on each other's output. Always run them with `asyncio.gather()`. Never await one before starting another.

---

## SoulBox Interaction Traps

### Tacet Reads SoulBox State — It Does Not Own It
Tacet reads SoulBox state as input to all three sub-agents. It never modifies SoulBox directly except through a formal evolution event. Never mutate SoulBox state as a side effect of processing.

```python
# bad — direct mutation
soulbox_state.personality_traits["warmth"] += 0.1

# good — formal evolution event
await soulbox.emit_evolution_event(EvolutionEvent(
    source=SubsystemId.TACET_HEART,
    trait="warmth",
    delta=0.1,
    reason="sustained positive interaction pattern"
))
```

### Evolution Events Are Logged — Always
Every SoulBox mutation triggered by Tacet must produce an evolution event that is logged to the episodic memory tier. This is the record the user can review and revert. Never mutate SoulBox silently.

### SoulBox Writes Are Atomic — Never Partial
An evolution event either applies fully or not at all. Never write partial SoulBox state. If a multi-trait evolution event fails midway, roll back all changes.

```python
# good — atomic via context manager
async with soulbox.atomic_update() as update:
    update.set_trait("warmth", new_warmth)
    update.set_trait("openness", new_openness)
# either both commit or neither does
```

---

## Persona Sub-Agent Traps

### Persona Reads From SoulBox on Every Call — Never Caches
Sena's voice evolves over time. Persona must read the current SoulBox personality snapshot on every call. Never cache personality state between calls.

### Persona Output Is Framing — Not Content
Persona determines how Sena speaks — tone, vocabulary, cadence, expressiveness. It does not determine what Sena says. Persona output feeds into prompt-composer as a framing layer, not as response content.

---

## Heart Sub-Agent Traps

### Heart Tracks Interaction Patterns — Not Single Events
Heart evolves Sena's relationship model based on patterns over time, not individual interactions. Never trigger a relationship evolution from a single data point.

```python
# bad — reacts to single interaction
if interaction.sentiment == "positive":
    await emit_evolution(trait="bond_strength", delta=0.1)

# good — pattern-based
pattern = await memory_agent.retrieve_pattern(
    PatternRequest(type="sentiment", window_days=7)
)
if pattern.sustained_positive():
    await emit_evolution(trait="bond_strength", delta=0.1)
```

### Heart Never Manufactures Emotion
Heart reflects genuine patterns in the user's interaction history. It must not fabricate warmth or depth that the interaction history does not support. Evolution events must always cite the memory evidence that triggered them.

---

## Reflection Sub-Agent Traps

### Reflection Feeds Into CTP — Not Directly to the User
Reflection output goes into CTP's consolidation cycle as a thought input. Reflection never surfaces content to the user directly.

### Reflection Runs During Idle — Never During Active Interaction
Deep reflection is resource-intensive. Reflection sub-agent should only run full cycles during idle states. During active interaction, Reflection provides lightweight identity consistency checks only.

```python
# good — depth based on activity state
if activity_state.is_deep_idle():
    await self.run_deep_reflection(context)
else:
    await self.run_consistency_check(context)
```

---

## Logging

Use `structlog` exclusively. Required fields on all Tacet log events:

- `sub_agent` — persona, heart, or reflection
- `event_type` — run_started, run_completed, evolution_emitted, soulbox_read
- `activity_state` — current activity level (relevant for Reflection depth)

Evolution events must be logged at `info` level with full trait, delta, and reason fields.
SoulBox write failures must be logged at `error` level with full rollback confirmation.
