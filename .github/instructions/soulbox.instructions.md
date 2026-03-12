---
applyTo: "soulbox/**"
---

# soulbox — Copilot Instructions

SoulBox is Sena's personalization and identity engine. It is the schema, storage, migration system, and event log for everything that makes Sena uniquely Sena for a specific user. It starts empty on first boot and evolves through interaction. It is intentionally Sena-specific and is not designed as an extractable subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Ownership Boundaries

SoulBox owns:
- The schema definition for all SoulBox state (TOML)
- Reading and exposing current SoulBox state to other subsystems via gRPC
- Accepting and applying evolution events from Tacet
- Versioned migration scripts — one per version increment
- The evolution event log — the complete history of how Sena has changed
- Backup creation before any migration

SoulBox does not own:
- Deciding when to evolve — that is Tacet/Heart
- Memory storage — that is memory-engine
- CTP behavior — SoulBox state influences CTP via config, not direct coupling
- Any UI rendering — SoulBox exposes data, never presents it

---

## Schema Traps

### SoulBox Schema Is TOML — Never JSON or Python Dicts
The source of truth for SoulBox state is a TOML file. Never represent SoulBox state as a Python dict or JSON blob in persistent storage.

```python
# bad — storing as JSON
with open("soulbox.json", "w") as f:
    json.dump(soulbox_state, f)

# good — TOML is the source of truth
with open("soulbox.toml", "wb") as f:
    tomllib_w.dump(soulbox_state, f)
```

### Schema Version Is Always Present
Every SoulBox TOML file must have a `schema_version` field at the root. Never write a SoulBox file without it. Never read a SoulBox file without checking it first.

```toml
# required — always present
schema_version = 3

[personality]
warmth = 0.6
openness = 0.7
```

```python
# good — always check version on read
state = tomllib.loads(raw)
if "schema_version" not in state:
    raise SoulBoxError("schema_version missing — file may be corrupted")
```

---

## Migration Traps

### Always Back Up Before Migrating
Before running any migration script, create a versioned backup of the current SoulBox file. Never migrate without a backup.

```python
# good — backup first, always
backup_path = create_versioned_backup(current_path, current_version)
logger.info("migration_backup_created", path=str(backup_path), version=current_version)
await run_migration(current_path, target_version)
```

### Migrations Are One Version at a Time
Never skip versions. v1 to v3 runs v1→v2 then v2→v3. Each migration script handles exactly one version increment.

```python
# bad — skips versions
await migrate_v1_to_v3(soulbox_path)

# good — sequential
for version in range(current_version, target_version):
    await migration_registry[version](soulbox_path)
```

### Migration Failure Restores From Backup
If any migration step raises an exception, immediately restore from the backup created before migration started. Never leave SoulBox in a partially migrated state.

```python
# good — restore on failure
try:
    await run_migration(path, version)
except MigrationError as error:
    logger.error("migration_failed", version=version, error=str(error))
    await restore_from_backup(backup_path, path)
    raise
```

### Migration Scripts Are Pure Functions
Each migration script takes the old state dict and returns the new state dict. No side effects, no file I/O inside the migration function itself. The migration runner handles reading and writing.

```python
# good — pure function
def migrate_v2_to_v3(state: dict) -> dict:
    new_state = deepcopy(state)
    new_state["schema_version"] = 3
    new_state["personality"]["curiosity"] = 0.5  # new field with default
    return new_state
```

---

## Evolution Event Traps

### Every Evolution Event Is Appended to the Log — Never Mutated
The evolution event log is append-only. Never modify or delete an existing evolution event. A correction is a new event, not an edit.

```python
# bad — mutates existing event
evolution_log[event_id].trait_delta = corrected_delta

# good — appends correction event
await evolution_log.append(EvolutionEvent(
    type=EventType.CORRECTION,
    corrects=event_id,
    trait=trait,
    delta=corrected_delta
))
```

### Evolution Events Must Be Reversible
Every evolution event must store enough information to be reverted. Always store the previous value alongside the new value.

```python
# bad — only stores new value
EvolutionEvent(trait="warmth", new_value=0.7)

# good — stores previous value for revertability
EvolutionEvent(
    trait="warmth",
    previous_value=current_warmth,
    new_value=0.7,
    source=SubsystemId.TACET_HEART,
    reason="sustained positive interaction pattern",
    timestamp=utcnow()
)
```

### Locked Traits Are Never Modified
If the user has locked a trait in SoulBox, no evolution event may modify it. Always check for locks before applying an evolution event.

```python
# good — check lock before applying
if soulbox_state.is_trait_locked(event.trait):
    logger.info("evolution_skipped_locked", trait=event.trait)
    return

await apply_evolution(event)
```

---

## Concurrency Traps

### SoulBox Writes Are Exclusive
SoulBox has a single `asyncio.Lock` for all writes. Only one write operation at a time. Multiple subsystems (Tacet, migration) may request writes — they must queue.

### Reads Are Concurrent — Never Block On Reads
Many subsystems read SoulBox state simultaneously (CTP, Tacet, PC, agents). Reads must never wait on each other. Use `asyncio.RLock` for reads or a reader-writer pattern.

---

## Logging

Use `structlog` exclusively. Required fields:

- `event_type` — schema_loaded, migration_started, migration_completed, migration_failed, backup_created, evolution_applied, evolution_skipped, trait_locked
- `schema_version` — always include on schema-related events
- `trait` — always include on evolution events

Migration events must be logged at `info` level at every step — start, backup created, each version step, completion or failure.
Evolution events must be logged at `info` level with full trait, delta, previous value, and source.
