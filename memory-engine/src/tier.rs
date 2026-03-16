//! Memory tier data containers.
//!
//! Defines `ShortTermTier`, `LongTermTier`, and `EpisodicTier` as thin structs
//! that hold their config slice and entry tracking state. These are the inner
//! types wrapped in `RwLock` inside `engine.rs` — one lock per tier, never one
//! lock for all.
//!
//! Tiers are data containers only. They hold local bookkeeping state that
//! memory-engine needs around ech0 operations (entry counts, ID tracking).
//! The actual storage and retrieval goes through ech0's `Store` — tiers never
//! duplicate that responsibility.
//!
//! `EpisodicTier` is **append-only**. It exposes no `update` or `delete`
//! methods. Corrections are new entries — existing episodic entries are
//! immutable once written.

use std::collections::HashSet;

use crate::config::{EpisodicTierConfig, LongTermTierConfig, ShortTermTierConfig};

// ─────────────────────────────────────────────────────────────────────────────
// Entry identifiers
// ─────────────────────────────────────────────────────────────────────────────

/// Opaque identifier for a memory entry within a tier.
///
/// Wraps a `String` so the type system prevents mixing entry IDs with
/// arbitrary strings. The inner value matches the ID assigned by ech0's
/// `IngestResult`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryId(pub String);

impl EntryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EntryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ShortTermTier
// ─────────────────────────────────────────────────────────────────────────────

/// Volatile, session-scoped memory tier.
///
/// Short-term entries live only for the duration of the current session.
/// CTP decides when to promote entries to long-term — memory-engine
/// executes the promotion but never decides when.
///
/// Cleared on shutdown. Never persisted directly.
#[derive(Debug)]
pub struct ShortTermTier {
    /// Configuration for this tier (max_entries, etc.).
    config: ShortTermTierConfig,

    /// Set of entry IDs currently tracked in this tier.
    /// Used for membership checks and to enforce `max_entries`.
    entry_ids: HashSet<EntryId>,
}

impl ShortTermTier {
    pub fn new(config: ShortTermTierConfig) -> Self {
        Self {
            config,
            entry_ids: HashSet::new(),
        }
    }

    /// Maximum number of entries this tier can hold.
    pub fn max_entries(&self) -> u32 {
        self.config.max_entries
    }

    /// Current number of entries tracked in this tier.
    pub fn entry_count(&self) -> usize {
        self.entry_ids.len()
    }

    /// Whether the tier has reached its configured capacity.
    pub fn is_full(&self) -> bool {
        self.entry_ids.len() >= self.config.max_entries as usize
    }

    /// Whether the tier tracks an entry with the given ID.
    pub fn contains(&self, entry_id: &EntryId) -> bool {
        self.entry_ids.contains(entry_id)
    }

    /// Track a new entry ID in this tier.
    ///
    /// Returns `true` if the entry was newly inserted, `false` if it was
    /// already present. The caller is responsible for checking `is_full()`
    /// before calling this — this method does not enforce the limit so that
    /// the caller can decide the eviction or rejection strategy.
    pub fn insert(&mut self, entry_id: EntryId) -> bool {
        self.entry_ids.insert(entry_id)
    }

    /// Remove an entry ID from this tier (e.g. after promotion to long-term).
    ///
    /// Returns `true` if the entry was present and removed, `false` if it
    /// was not tracked.
    pub fn remove(&mut self, entry_id: &EntryId) -> bool {
        self.entry_ids.remove(entry_id)
    }

    /// Remove all tracked entries. Called on session end / shutdown.
    pub fn clear(&mut self) {
        self.entry_ids.clear();
    }

    /// Return a snapshot of all currently tracked entry IDs.
    pub fn entry_ids(&self) -> &HashSet<EntryId> {
        &self.entry_ids
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// LongTermTier
// ─────────────────────────────────────────────────────────────────────────────

/// Persistent memory tier. Entries are promoted here from short-term by CTP's
/// decision, executed by memory-engine.
///
/// Long-term entries survive across sessions and are subject to importance
/// decay (via ech0's `importance-decay` feature). The decay floor from config
/// ensures no entry ever reaches zero weight.
#[derive(Debug)]
pub struct LongTermTier {
    /// Configuration for this tier (max_entries, etc.).
    config: LongTermTierConfig,

    /// Set of entry IDs currently tracked in this tier.
    entry_ids: HashSet<EntryId>,
}

impl LongTermTier {
    pub fn new(config: LongTermTierConfig) -> Self {
        Self {
            config,
            entry_ids: HashSet::new(),
        }
    }

    /// Maximum number of entries this tier can hold.
    pub fn max_entries(&self) -> u32 {
        self.config.max_entries
    }

    /// Current number of entries tracked in this tier.
    pub fn entry_count(&self) -> usize {
        self.entry_ids.len()
    }

    /// Whether the tier has reached its configured capacity.
    pub fn is_full(&self) -> bool {
        self.entry_ids.len() >= self.config.max_entries as usize
    }

    /// Whether the tier tracks an entry with the given ID.
    pub fn contains(&self, entry_id: &EntryId) -> bool {
        self.entry_ids.contains(entry_id)
    }

    /// Track a new entry ID in this tier.
    ///
    /// Returns `true` if the entry was newly inserted, `false` if it was
    /// already present. The caller is responsible for checking `is_full()`
    /// before calling this.
    pub fn insert(&mut self, entry_id: EntryId) -> bool {
        self.entry_ids.insert(entry_id)
    }

    /// Remove an entry ID from this tier.
    ///
    /// Returns `true` if the entry was present and removed, `false` if it
    /// was not tracked.
    pub fn remove(&mut self, entry_id: &EntryId) -> bool {
        self.entry_ids.remove(entry_id)
    }

    /// Return a snapshot of all currently tracked entry IDs.
    pub fn entry_ids(&self) -> &HashSet<EntryId> {
        &self.entry_ids
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EpisodicTier
// ─────────────────────────────────────────────────────────────────────────────

/// Append-only episodic memory tier.
///
/// Episodic entries represent discrete events, experiences, or observations.
/// Once written, they are **never mutated or deleted**. Corrections to
/// episodic memory are expressed as new entries — the original entry remains
/// intact for provenance.
///
/// This type intentionally exposes **no `update`, `delete`, `remove`, or
/// `clear` methods**. The append-only invariant is enforced by the type
/// itself — not by runtime checks that could be bypassed.
#[derive(Debug)]
pub struct EpisodicTier {
    /// Configuration for this tier (max_entries, etc.).
    config: EpisodicTierConfig,

    /// Ordered list of entry IDs in insertion order.
    /// A `Vec` is used instead of `HashSet` to preserve chronological order,
    /// which is semantically meaningful for episodic memory.
    entry_ids: Vec<EntryId>,

    /// Fast membership lookup to avoid O(n) scans on the Vec.
    entry_id_set: HashSet<EntryId>,
}

impl EpisodicTier {
    pub fn new(config: EpisodicTierConfig) -> Self {
        Self {
            config,
            entry_ids: Vec::new(),
            entry_id_set: HashSet::new(),
        }
    }

    /// Maximum number of entries this tier can hold.
    pub fn max_entries(&self) -> u32 {
        self.config.max_entries
    }

    /// Current number of entries tracked in this tier.
    pub fn entry_count(&self) -> usize {
        self.entry_ids.len()
    }

    /// Whether the tier has reached its configured capacity.
    pub fn is_full(&self) -> bool {
        self.entry_ids.len() >= self.config.max_entries as usize
    }

    /// Whether the tier tracks an entry with the given ID.
    pub fn contains(&self, entry_id: &EntryId) -> bool {
        self.entry_id_set.contains(entry_id)
    }

    /// Append a new entry ID to this tier.
    ///
    /// Returns `true` if the entry was newly appended, `false` if it was
    /// already present (episodic entries are unique — duplicate appends are
    /// idempotent, not errors). The caller is responsible for checking
    /// `is_full()` before calling this.
    ///
    /// This is the **only** mutation method on `EpisodicTier`. There is no
    /// `update`, `delete`, `remove`, or `clear` — by design.
    pub fn append(&mut self, entry_id: EntryId) -> bool {
        if self.entry_id_set.contains(&entry_id) {
            return false;
        }
        self.entry_id_set.insert(entry_id.clone());
        self.entry_ids.push(entry_id);
        true
    }

    /// Return the entry IDs in chronological insertion order.
    pub fn entry_ids_ordered(&self) -> &[EntryId] {
        &self.entry_ids
    }

    /// Return the entry ID set for fast membership checks.
    pub fn entry_ids(&self) -> &HashSet<EntryId> {
        &self.entry_id_set
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tier name constants — used in structured log fields
// ─────────────────────────────────────────────────────────────────────────────

/// Tier name constants for use in structured logging fields.
/// These are the only valid values for the `tier` field in log events.
pub mod tier_name {
    pub const SHORT_TERM: &str = "short_term";
    pub const LONG_TERM: &str = "long_term";
    pub const EPISODIC: &str = "episodic";
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn short_term_config() -> ShortTermTierConfig {
        ShortTermTierConfig { max_entries: 3 }
    }

    fn long_term_config() -> LongTermTierConfig {
        LongTermTierConfig { max_entries: 5 }
    }

    fn episodic_config() -> EpisodicTierConfig {
        EpisodicTierConfig { max_entries: 4 }
    }

    // ── ShortTermTier ───────────────────────────────────────────────────

    #[test]
    fn short_term_starts_empty() {
        let tier = ShortTermTier::new(short_term_config());
        assert_eq!(tier.entry_count(), 0);
        assert!(!tier.is_full());
    }

    #[test]
    fn short_term_insert_and_contains() {
        let mut tier = ShortTermTier::new(short_term_config());
        let entry_id = EntryId::new("entry-1");

        assert!(tier.insert(entry_id.clone()));
        assert!(tier.contains(&entry_id));
        assert_eq!(tier.entry_count(), 1);
    }

    #[test]
    fn short_term_duplicate_insert_returns_false() {
        let mut tier = ShortTermTier::new(short_term_config());
        let entry_id = EntryId::new("entry-1");

        assert!(tier.insert(entry_id.clone()));
        assert!(!tier.insert(entry_id));
        assert_eq!(tier.entry_count(), 1);
    }

    #[test]
    fn short_term_is_full_at_capacity() {
        let mut tier = ShortTermTier::new(short_term_config());

        tier.insert(EntryId::new("a"));
        tier.insert(EntryId::new("b"));
        assert!(!tier.is_full());

        tier.insert(EntryId::new("c"));
        assert!(tier.is_full());
    }

    #[test]
    fn short_term_remove() {
        let mut tier = ShortTermTier::new(short_term_config());
        let entry_id = EntryId::new("entry-1");

        tier.insert(entry_id.clone());
        assert!(tier.remove(&entry_id));
        assert!(!tier.contains(&entry_id));
        assert_eq!(tier.entry_count(), 0);
    }

    #[test]
    fn short_term_remove_missing_returns_false() {
        let mut tier = ShortTermTier::new(short_term_config());
        assert!(!tier.remove(&EntryId::new("nonexistent")));
    }

    #[test]
    fn short_term_clear() {
        let mut tier = ShortTermTier::new(short_term_config());
        tier.insert(EntryId::new("a"));
        tier.insert(EntryId::new("b"));

        tier.clear();
        assert_eq!(tier.entry_count(), 0);
        assert!(!tier.contains(&EntryId::new("a")));
    }

    // ── LongTermTier ────────────────────────────────────────────────────

    #[test]
    fn long_term_starts_empty() {
        let tier = LongTermTier::new(long_term_config());
        assert_eq!(tier.entry_count(), 0);
        assert!(!tier.is_full());
    }

    #[test]
    fn long_term_insert_and_contains() {
        let mut tier = LongTermTier::new(long_term_config());
        let entry_id = EntryId::new("lt-1");

        assert!(tier.insert(entry_id.clone()));
        assert!(tier.contains(&entry_id));
    }

    #[test]
    fn long_term_is_full_at_capacity() {
        let mut tier = LongTermTier::new(LongTermTierConfig { max_entries: 2 });

        tier.insert(EntryId::new("a"));
        tier.insert(EntryId::new("b"));
        assert!(tier.is_full());
    }

    #[test]
    fn long_term_remove() {
        let mut tier = LongTermTier::new(long_term_config());
        let entry_id = EntryId::new("lt-1");

        tier.insert(entry_id.clone());
        assert!(tier.remove(&entry_id));
        assert!(!tier.contains(&entry_id));
    }

    // ── EpisodicTier ────────────────────────────────────────────────────

    #[test]
    fn episodic_starts_empty() {
        let tier = EpisodicTier::new(episodic_config());
        assert_eq!(tier.entry_count(), 0);
        assert!(!tier.is_full());
    }

    #[test]
    fn episodic_append_and_contains() {
        let mut tier = EpisodicTier::new(episodic_config());
        let entry_id = EntryId::new("ep-1");

        assert!(tier.append(entry_id.clone()));
        assert!(tier.contains(&entry_id));
        assert_eq!(tier.entry_count(), 1);
    }

    #[test]
    fn episodic_duplicate_append_returns_false() {
        let mut tier = EpisodicTier::new(episodic_config());
        let entry_id = EntryId::new("ep-1");

        assert!(tier.append(entry_id.clone()));
        assert!(!tier.append(entry_id));
        assert_eq!(tier.entry_count(), 1);
    }

    #[test]
    fn episodic_preserves_insertion_order() {
        let mut tier = EpisodicTier::new(episodic_config());

        tier.append(EntryId::new("first"));
        tier.append(EntryId::new("second"));
        tier.append(EntryId::new("third"));

        let ordered = tier.entry_ids_ordered();
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0].as_str(), "first");
        assert_eq!(ordered[1].as_str(), "second");
        assert_eq!(ordered[2].as_str(), "third");
    }

    #[test]
    fn episodic_is_full_at_capacity() {
        let mut tier = EpisodicTier::new(EpisodicTierConfig { max_entries: 2 });

        tier.append(EntryId::new("a"));
        tier.append(EntryId::new("b"));
        assert!(tier.is_full());
    }

    // ── EntryId ─────────────────────────────────────────────────────────

    #[test]
    fn entry_id_display() {
        let entry_id = EntryId::new("test-id-123");
        assert_eq!(format!("{}", entry_id), "test-id-123");
    }

    #[test]
    fn entry_id_equality() {
        let id_a = EntryId::new("same");
        let id_b = EntryId::new("same");
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn entry_id_inequality() {
        let id_a = EntryId::new("one");
        let id_b = EntryId::new("two");
        assert_ne!(id_a, id_b);
    }
}
