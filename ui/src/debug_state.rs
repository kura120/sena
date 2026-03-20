use std::collections::HashMap;
use std::collections::VecDeque;

use chrono::{DateTime, Utc};

/// Status of a subsystem as observed from daemon-bus events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemHealthStatus {
    Unknown,
    Ready,
    Degraded,
    Unavailable,
}

impl Default for SubsystemHealthStatus {
    fn default() -> Self {
        SubsystemHealthStatus::Unknown
    }
}

impl std::fmt::Display for SubsystemHealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubsystemHealthStatus::Unknown => write!(f, "Unknown"),
            SubsystemHealthStatus::Ready => write!(f, "Ready"),
            SubsystemHealthStatus::Degraded => write!(f, "Degraded"),
            SubsystemHealthStatus::Unavailable => write!(f, "Unavailable"),
        }
    }
}

/// A surfaced CTP thought.
#[derive(Debug, Clone)]
pub struct ThoughtEvent {
    pub content: String,
    pub relevance_score: f32,
    pub timestamp: DateTime<Utc>,
}

/// Stats for the three memory tiers.
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub short_term_count: u32,
    pub long_term_count: u32,
    pub episodic_count: u32,
    pub last_write: Option<DateTime<Utc>>,
}

impl Default for MemoryStats {
    fn default() -> Self {
        Self {
            short_term_count: 0,
            long_term_count: 0,
            episodic_count: 0,
            last_write: None,
        }
    }
}

/// A raw daemon-bus event for the event feed.
#[derive(Debug, Clone)]
pub struct BusEventEntry {
    pub topic: String,
    pub source_subsystem: String,
    pub payload_summary: String,
    pub timestamp: DateTime<Utc>,
}

/// A boot signal received from daemon-bus, for the boot signal history panel.
#[derive(Debug, Clone)]
pub struct BootSignalEntry {
    pub signal_name: String,
    pub source_subsystem: String,
    pub required: bool,
    pub timestamp: DateTime<Utc>,
}

/// All known subsystem identifiers in Sena.
/// Keys always use hyphens — never underscores.
pub const KNOWN_SUBSYSTEMS: &[&str] = &[
    "daemon-bus",
    "memory-engine",
    "inference",
    "model-probe",
    "lora-manager",
    "ctp",
    "prompt-composer",
    "soulbox",
    "agents",
    "platform",
    "ui",
];

/// Boot signals that are required for Sena to reach SENA_READY.
pub const REQUIRED_BOOT_SIGNALS: &[&str] = &[
    "DAEMON_BUS_READY",
    "MEMORY_ENGINE_READY",
    "PLATFORM_READY",
    "AGENTS_READY",
    "INFERENCE_READY",
    "SOULBOX_READY",
    "PROMPT_COMPOSER_READY",
    "CTP_READY",
];

/// All state for the debug panel, updated from daemon-bus event streams.
#[derive(Debug, Clone)]
pub struct DebugState {
    pub subsystem_health: HashMap<String, SubsystemHealthStatus>,
    pub vram_used_mb: u32,
    pub vram_total_mb: u32,
    pub thought_feed: VecDeque<ThoughtEvent>,
    pub thought_feed_max: usize,
    pub memory_stats: MemoryStats,
    pub event_feed: VecDeque<BusEventEntry>,
    pub event_feed_max: usize,
    pub connected: bool,
    pub boot_signal_history: VecDeque<BootSignalEntry>,
    pub boot_signal_history_max: usize,
    pub total_events_received: u64,
    pub started_at: Option<DateTime<Utc>>,
}

impl DebugState {
    /// Create a new DebugState with the given capacity limits.
    pub fn new(thought_feed_max: usize, event_feed_max: usize) -> Self {
        let mut subsystem_health = HashMap::new();
        for name in KNOWN_SUBSYSTEMS {
            subsystem_health.insert((*name).to_string(), SubsystemHealthStatus::Unknown);
        }
        Self {
            subsystem_health,
            vram_used_mb: 0,
            vram_total_mb: 0,
            thought_feed: VecDeque::with_capacity(thought_feed_max),
            thought_feed_max,
            memory_stats: MemoryStats::default(),
            event_feed: VecDeque::with_capacity(event_feed_max),
            event_feed_max,
            connected: false,
            boot_signal_history: VecDeque::with_capacity(100),
            boot_signal_history_max: 100,
            total_events_received: 0,
            started_at: None,
        }
    }

    /// Push a thought event, enforcing the max capacity.
    pub fn push_thought(&mut self, thought: ThoughtEvent) {
        if self.thought_feed.len() >= self.thought_feed_max {
            self.thought_feed.pop_back();
        }
        self.thought_feed.push_front(thought);
    }

    /// Push a bus event, enforcing the max capacity.
    pub fn push_event(&mut self, event: BusEventEntry) {
        if self.event_feed.len() >= self.event_feed_max {
            self.event_feed.pop_back();
        }
        self.event_feed.push_front(event);
    }

    /// Push a boot signal entry, enforcing the max capacity.
    pub fn push_boot_signal(&mut self, entry: BootSignalEntry) {
        if self.boot_signal_history.len() >= self.boot_signal_history_max {
            self.boot_signal_history.pop_back();
        }
        self.boot_signal_history.push_front(entry);
    }

    /// Update subsystem health status.
    pub fn set_subsystem_status(&mut self, name: &str, status: SubsystemHealthStatus) {
        self.subsystem_health
            .insert(name.to_string(), status);
    }
}

impl Default for DebugState {
    fn default() -> Self {
        Self::new(100, 200)
    }
}

/// Format a timestamp as a human-readable relative time string.
pub fn format_relative_time(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(timestamp);

    if duration.num_seconds() < 0 {
        return "just now".to_string();
    }

    let seconds = duration.num_seconds();
    if seconds < 2 {
        return "just now".to_string();
    }
    if seconds < 60 {
        return format!("{seconds}s ago");
    }
    let minutes = duration.num_minutes();
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = duration.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = duration.num_days();
    format!("{days}d ago")
}

/// Format an uptime duration as a human-readable string.
pub fn format_uptime(started_at: Option<DateTime<Utc>>) -> String {
    let Some(start) = started_at else {
        return "—".to_string();
    };
    let duration = Utc::now().signed_duration_since(start);
    let total_seconds = duration.num_seconds().max(0);
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Format a topic integer value to a human-readable name.
pub fn format_topic_name(topic_value: i32) -> &'static str {
    match topic_value {
        0 => "Unspecified",
        1 => "Boot Signal",
        2 => "Boot Failed",
        10 => "Subsystem Started",
        11 => "Subsystem Crashed",
        12 => "Subsystem Restarted",
        13 => "Subsystem Degraded",
        20 => "Escalation Granted",
        21 => "Escalation Queued",
        22 => "Escalation Expired",
        23 => "Escalation Released",
        30 => "Task Timeout",
        31 => "Task Terminated",
        40 => "Memory Updated",
        41 => "Memory Write Completed",
        42 => "Memory Tier Promoted",
        50 => "Model Probe Failed",
        51 => "LoRA Training Recommended",
        60 => "User Message Received",
        61 => "User Message Response",
        62 => "Thought Surfaced",
        63 => "Session Compaction Triggered",
        64 => "Memory Consolidation Requested",
        65 => "Inference Model Switching",
        66 => "Agent Registered",
        67 => "Agent Quarantined",
        _ => "Unknown Topic",
    }
}

/// Truncate a string to the given max length, appending an ellipsis if truncated.
pub fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_state_default_all_unknown() {
        let state = DebugState::default();
        for name in KNOWN_SUBSYSTEMS {
            let status = state.subsystem_health.get(*name);
            assert_eq!(
                status,
                Some(&SubsystemHealthStatus::Unknown),
                "subsystem {name} should default to Unknown"
            );
        }
        assert!(!state.connected);
    }

    #[test]
    fn test_debug_state_update_subsystem_ready() {
        let mut state = DebugState::default();
        state.set_subsystem_status("memory-engine", SubsystemHealthStatus::Ready);
        assert_eq!(
            state.subsystem_health.get("memory-engine"),
            Some(&SubsystemHealthStatus::Ready)
        );
    }

    #[test]
    fn test_debug_state_update_thought_surfaced() {
        let mut state = DebugState::default();
        let thought = ThoughtEvent {
            content: "test thought".to_string(),
            relevance_score: 0.85,
            timestamp: Utc::now(),
        };
        state.push_thought(thought.clone());
        assert_eq!(state.thought_feed.len(), 1);
        assert!((state.thought_feed[0].relevance_score - 0.85).abs() < f32::EPSILON);
    }

    #[test]
    fn test_debug_state_event_feed_max_capacity() {
        let mut state = DebugState::new(100, 5);
        for i in 0..10 {
            state.push_event(BusEventEntry {
                topic: format!("topic_{i}"),
                source_subsystem: "test".to_string(),
                payload_summary: String::new(),
                timestamp: Utc::now(),
            });
        }
        assert_eq!(
            state.event_feed.len(),
            5,
            "event feed should be capped at max capacity"
        );
    }

    #[test]
    fn test_thought_feed_max_capacity() {
        let mut state = DebugState::new(3, 200);
        for i in 0..10 {
            state.push_thought(ThoughtEvent {
                content: format!("thought_{i}"),
                relevance_score: 0.5,
                timestamp: Utc::now(),
            });
        }
        assert_eq!(
            state.thought_feed.len(),
            3,
            "thought feed should be capped at max capacity"
        );
    }

    #[test]
    fn test_format_topic_name_known() {
        assert_eq!(format_topic_name(1), "Boot Signal");
        assert_eq!(format_topic_name(62), "Thought Surfaced");
        assert_eq!(format_topic_name(40), "Memory Updated");
    }

    #[test]
    fn test_format_topic_name_unknown() {
        assert_eq!(format_topic_name(999), "Unknown Topic");
    }

    #[test]
    fn test_truncate_with_ellipsis_short() {
        assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_with_ellipsis_long() {
        let long_text = "a".repeat(80);
        let result = truncate_with_ellipsis(&long_text, 60);
        assert!(result.len() <= 63); // 59 chars + multibyte ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_format_relative_time_just_now() {
        let now = Utc::now();
        assert_eq!(format_relative_time(now), "just now");
    }

    #[test]
    fn test_format_relative_time_seconds() {
        let past = Utc::now() - chrono::Duration::seconds(30);
        let result = format_relative_time(past);
        assert!(result.contains("s ago"), "expected seconds ago, got: {result}");
    }

    #[test]
    fn test_format_relative_time_minutes() {
        let past = Utc::now() - chrono::Duration::minutes(5);
        let result = format_relative_time(past);
        assert!(result.contains("m ago"), "expected minutes ago, got: {result}");
    }

    #[test]
    fn test_push_boot_signal_capacity() {
        let mut state = DebugState::default();
        for i in 0..110 {
            state.push_boot_signal(BootSignalEntry {
                signal_name: format!("SIGNAL_{i}"),
                source_subsystem: "test".to_string(),
                required: i % 2 == 0,
                timestamp: Utc::now(),
            });
        }
        assert_eq!(state.boot_signal_history.len(), 100);
        // Newest at front
        assert_eq!(state.boot_signal_history[0].signal_name, "SIGNAL_109");
    }

    #[test]
    fn test_format_uptime_none() {
        assert_eq!(format_uptime(None), "—");
    }

    #[test]
    fn test_format_uptime_seconds() {
        let started = Utc::now() - chrono::Duration::seconds(45);
        let result = format_uptime(Some(started));
        assert!(result.contains("s"), "expected seconds format, got: {result}");
    }

    #[test]
    fn test_required_boot_signals_not_empty() {
        assert!(!REQUIRED_BOOT_SIGNALS.is_empty());
    }
}
