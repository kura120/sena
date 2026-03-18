---
applyTo: "soulbox/**"
---

# soulbox — Copilot Instructions

SoulBox is Sena's personalization and identity engine. It is the schema, storage, migration system, and event log for everything that makes Sena uniquely Sena for a specific user. It starts empty on first boot and evolves through interaction. It is intentionally Sena-specific and is not designed as an extractable subsystem.

These rules are traps specific to this subsystem. Global rules in `.github/copilot-instructions.md` also apply in full.

---

## Language

**Rust only.** SoulBox is security-critical — Argon2id, AES-256-GCM, Windows DPAPI. No Python. All async is tokio. All locking is `tokio::sync::RwLock`. Serialization uses the `toml` crate for the on-disk format and `serde` for all struct mapping.

---

## Ownership Boundaries

SoulBox owns:
- The schema definition for all SoulBox state (TOML on disk, serde structs in memory)
- Encryption and decryption of the SoulBox file — Argon2id key derivation, AES-256-GCM encryption, Windows DPAPI wrapping of the derived key
- Reading and exposing current SoulBox state to other subsystems via gRPC
- Accepting and applying evolution events from Tacet
- Versioned migration scripts — one per version increment, living in `soulbox/migrations/`
- The evolution event log — append-only, complete history of how Sena has changed
- Backup creation before any migration
- The agent registry encryption context — separate file, separate key, same password derivation

SoulBox does not own:
- Deciding when to evolve — that is Tacet/Heart
- Memory storage — that is memory-engine
- CTP behavior — SoulBox state influences CTP via config, not direct coupling
- Any UI rendering — SoulBox exposes data, never presents it
- The agent registry content — that is the agents subsystem

---

## Schema Traps

### SoulBox Schema Is TOML — Never JSON

The source of truth for SoulBox state is an encrypted TOML file. Never represent SoulBox state as JSON in persistent storage.

```rust
// bad — serialized as JSON to disk
let json = serde_json::to_string(&soulbox_state)?;
std::fs::write("soulbox.json", json)?;

// good — TOML is the on-disk format
let toml_str = toml::to_string(&soulbox_state)?;
encrypt_and_write("soulbox.toml.enc", toml_str.as_bytes(), &key)?;
```

### Schema Version Is Always Present

Every SoulBox struct must carry `schema_version: u32`. Never write a SoulBox file without it. Never deserialize a SoulBox file without checking it first.

```rust
// required — always present and always checked
#[derive(Serialize, Deserialize)]
pub struct SoulBoxState {
    pub schema_version: u32,
    pub personality: PersonalityTraits,
    // ...
}

fn load(raw: &str) -> Result<SoulBoxState, SoulBoxError> {
    let state: SoulBoxState = toml::from_str(raw)?;
    if state.schema_version == 0 {
        return Err(SoulBoxError::MissingSchemaVersion);
    }
    Ok(state)
}
```

---

## Encryption Traps

### Key Derivation Uses Argon2id — No Exceptions

The encryption key is always derived from the user's SoulBox password using Argon2id. Never use a hardcoded key, a randomly generated key stored in plaintext, or any other KDF.

### The Derived Key Is Wrapped With Windows DPAPI

After key derivation, the derived key is wrapped using Windows DPAPI before being stored on disk. Never store the raw derived key anywhere. DPAPI unwrapping happens at load time, not at derivation time.

### Nonces Are Never Reused

Every AES-256-GCM encryption operation uses a freshly generated random nonce. Never reuse a nonce. Store the nonce alongside the ciphertext.

### The Agent Registry Is a Separate Encryption Context

The agent registry (`agent-registry.toml.enc`) uses the same password and the same Argon2id parameters, but a **different salt** and therefore a **different derived key**. Never share the SoulBox encryption key with the agent registry. They are separate files, separate keys, separate failure domains.

---

## Migration Traps

### Always Back Up Before Migrating

Before running any migration, create a versioned backup of the current encrypted SoulBox file. Never migrate without a confirmed backup.

```rust
// good — backup first, always
let backup_path = create_versioned_backup(&current_path, current_version).await?;
tracing::info!(
    event_type = "migration_backup_created",
    path = %backup_path.display(),
    schema_version = current_version,
);
run_migration(&current_path, target_version).await?;
```

### Migrations Are One Version at a Time

Never skip versions. v1 → v3 runs v1→v2 then v2→v3. Each migration function handles exactly one version increment.

```rust
// bad — skips versions
migrate_v1_to_v3(&mut state)?;

// good — sequential
for version in current_version..target_version {
    let migrate = migration_registry(version)?;
    migrate(&mut state)?;
}
```

### Migration Failure Restores From Backup

If any migration step returns an error, immediately restore from the backup created before migration started. Never leave SoulBox in a partially migrated state.

```rust
match run_migration(&path, target_version).await {
    Ok(_) => {},
    Err(error) => {
        tracing::error!(event_type = "migration_failed", schema_version = target_version, %error);
        restore_from_backup(&backup_path, &path).await?;
        return Err(error.into());
    }
}
```

### Migration Functions Are Pure — No I/O

Each migration function takes the old `SoulBoxState` and returns a new `SoulBoxState`. No file I/O, no gRPC calls, no side effects inside the migration function itself. The migration runner handles reading and writing.

```rust
// good — pure transformation
fn migrate_v2_to_v3(mut state: SoulBoxState) -> Result<SoulBoxState, MigrationError> {
    state.schema_version = 3;
    state.personality.curiosity = Some(0.5); // new field, default value
    Ok(state)
}
```

---

## Evolution Event Traps

### The Evolution Log Is Append-Only — Never Mutated

Never modify or delete an existing evolution event. A correction is a new event, not an edit.

```rust
// bad — mutates existing event
evolution_log[event_id].trait_delta = corrected_delta;

// good — appends a correction event
evolution_log.append(EvolutionEvent {
    event_type: EventType::Correction,
    corrects: Some(event_id),
    trait_name: trait_name.clone(),
    delta: corrected_delta,
    ..EvolutionEvent::now()
})?;
```

### Evolution Events Must Be Reversible

Every event stores the previous value alongside the new value.

```rust
// bad — only stores new value
EvolutionEvent { trait_name: "warmth".into(), new_value: 0.7, ..Default::default() }

// good — stores previous value for revertability
EvolutionEvent {
    trait_name: "warmth".into(),
    previous_value: current_warmth,
    new_value: 0.7,
    source: SubsystemId::TacetHeart,
    reason: "sustained positive interaction pattern".into(),
    timestamp: Utc::now(),
}
```

### Locked Traits Are Never Modified

If the user has locked a trait, no evolution event may modify it. Always check the lock before applying.

```rust
if soulbox_state.is_trait_locked(&event.trait_name) {
    tracing::info!(event_type = "evolution_skipped_locked", trait = %event.trait_name);
    return Ok(());
}
apply_evolution(&mut soulbox_state, &event)?;
```

---

## Concurrency Traps

### Writes Are Exclusive — Use `RwLock` Write Guard

SoulBox state is held behind a `tokio::sync::RwLock<SoulBoxState>`. Only one write at a time. Never hold the write guard across an `await` point.

```rust
// bad — holds write guard across await
let mut state = soulbox.write().await;
state.personality.warmth = new_warmth;
some_async_call().await; // write guard still held
drop(state);

// good — drop guard before awaiting
{
    let mut state = soulbox.write().await;
    state.personality.warmth = new_warmth;
} // guard dropped here
some_async_call().await;
```

### Reads Are Concurrent — Use `RwLock` Read Guard

Many subsystems read SoulBox state simultaneously (CTP, Tacet, prompt-composer). Read guards never block each other. Never take a write guard when a read guard is sufficient.

---

## Logging

Use `tracing` exclusively. Required fields:

- `event_type` — `schema_loaded`, `migration_started`, `migration_completed`, `migration_failed`, `backup_created`, `evolution_applied`, `evolution_skipped`, `trait_locked`
- `schema_version` — always include on schema-related events
- `trait` — always include on evolution events

Migration events: `info` level at every step — start, backup created, each version step, completion or failure.
Evolution events: `info` level with full trait, delta, previous value, and source.
Encryption errors: `error` level — never swallow them.