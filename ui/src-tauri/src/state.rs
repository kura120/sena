use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// Subsystem health status for UI display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SubsystemHealthStatus {
    Unknown,
    Ready,
    Degraded,
    Unavailable,
}

impl Default for SubsystemHealthStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

/// A thought event surfaced by CTP
#[derive(Debug, Clone, Serialize)]
pub struct ThoughtEvent {
    pub content: String,
    pub relevance_score: f32,
    pub timestamp: DateTime<Utc>,
}

/// Memory statistics across all three tiers
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryStats {
    pub short_term_count: u32,
    pub long_term_count: u32,
    pub episodic_count: u32,
    pub last_write: Option<DateTime<Utc>>,
}

/// A bus event entry for the event feed
#[derive(Debug, Clone, Serialize)]
pub struct BusEventEntry {
    pub topic: String,
    pub source_subsystem: String,
    pub payload_summary: String,
    pub timestamp: DateTime<Utc>,
}

/// A boot signal entry for the boot timeline
#[derive(Debug, Clone, Serialize)]
pub struct BootSignalEntry {
    pub signal_name: String,
    pub source_subsystem: String,
    pub required: bool,
    pub timestamp: DateTime<Utc>,
}

/// All known subsystems in Sena
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

/// Required boot signals for Sena to be considered fully ready
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

/// Central debug state shared between gRPC event handler and Tauri commands
#[derive(Debug)]
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
    pub fn new(thought_feed_max: usize, event_feed_max: usize) -> Self {
        let mut subsystem_health = HashMap::new();
        for subsystem in KNOWN_SUBSYSTEMS {
            subsystem_health.insert(subsystem.to_string(), SubsystemHealthStatus::Unknown);
        }

        Self {
            subsystem_health,
            vram_used_mb: 0,
            vram_total_mb: 0,
            thought_feed: VecDeque::new(),
            thought_feed_max,
            memory_stats: MemoryStats::default(),
            event_feed: VecDeque::new(),
            event_feed_max,
            connected: false,
            boot_signal_history: VecDeque::new(),
            boot_signal_history_max: 100,
            total_events_received: 0,
            started_at: None,
        }
    }

    /// Set subsystem health status
    pub fn set_subsystem_status(&mut self, subsystem: &str, status: SubsystemHealthStatus) {
        self.subsystem_health.insert(subsystem.to_string(), status);
    }

    /// Push a thought event to the feed, maintaining capacity limit
    pub fn push_thought(&mut self, thought: ThoughtEvent) {
        self.thought_feed.push_front(thought);
        while self.thought_feed.len() > self.thought_feed_max {
            self.thought_feed.pop_back();
        }
    }

    /// Push an event to the event feed, maintaining capacity limit
    pub fn push_event(&mut self, event: BusEventEntry) {
        self.event_feed.push_front(event);
        while self.event_feed.len() > self.event_feed_max {
            self.event_feed.pop_back();
        }
    }

    /// Push a boot signal to the history, maintaining capacity limit
    pub fn push_boot_signal(&mut self, signal: BootSignalEntry) {
        self.boot_signal_history.push_front(signal);
        while self.boot_signal_history.len() > self.boot_signal_history_max {
            self.boot_signal_history.pop_back();
        }
    }

    /// Update memory statistics based on tier and count
    pub fn update_memory_stats(&mut self, tier: &str, count: u32) {
        match tier {
            "short_term" => self.memory_stats.short_term_count = count,
            "long_term" => self.memory_stats.long_term_count = count,
            "episodic" => self.memory_stats.episodic_count = count,
            _ => {}
        }
        self.memory_stats.last_write = Some(Utc::now());
    }
}

/// Format a timestamp as a relative time string (e.g., "2 minutes ago")
pub fn format_relative_time(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(timestamp);

    if duration < Duration::zero() {
        return "just now".to_string();
    }

    if duration < Duration::seconds(60) {
        return format!("{}s ago", duration.num_seconds());
    }

    if duration < Duration::minutes(60) {
        let mins = duration.num_minutes();
        return format!("{}m ago", mins);
    }

    if duration < Duration::hours(24) {
        let hours = duration.num_hours();
        return format!("{}h ago", hours);
    }

    let days = duration.num_days();
    format!("{}d ago", days)
}

/// Format an uptime duration (e.g., "2h 15m 30s")
pub fn format_uptime(started_at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(started_at);

    if duration < Duration::zero() {
        return "0s".to_string();
    }

    let total_seconds = duration.num_seconds();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Map EventTopic i32 values to human-readable names
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
        70 => "PC Prompt Assembled",
        _ => "Unknown Topic",
    }
}

/// Truncate a string to max_len with ellipsis
pub fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_state_default_all_unknown() {
        let state = DebugState::new(100, 200);

        // All known subsystems should start as Unknown
        for subsystem in KNOWN_SUBSYSTEMS {
            assert_eq!(
                state.subsystem_health.get(*subsystem),
                Some(&SubsystemHealthStatus::Unknown)
            );
        }

        assert_eq!(state.thought_feed.len(), 0);
        assert_eq!(state.event_feed.len(), 0);
        assert_eq!(state.boot_signal_history.len(), 0);
        assert!(!state.connected);
        assert_eq!(state.total_events_received, 0);
        assert!(state.started_at.is_none());
    }

    #[test]
    fn test_debug_state_update_subsystem_ready() {
        let mut state = DebugState::new(100, 200);

        state.set_subsystem_status("daemon-bus", SubsystemHealthStatus::Ready);

        assert_eq!(
            state.subsystem_health.get("daemon-bus"),
            Some(&SubsystemHealthStatus::Ready)
        );

        // Other subsystems should still be Unknown
        assert_eq!(
            state.subsystem_health.get("inference"),
            Some(&SubsystemHealthStatus::Unknown)
        );
    }

    #[test]
    fn test_debug_state_push_event_capacity() {
        let mut state = DebugState::new(100, 5);

        // Push 10 events, but capacity is 5
        for i in 0..10 {
            state.push_event(BusEventEntry {
                topic: format!("Topic {}", i),
                source_subsystem: "test".to_string(),
                payload_summary: "test payload".to_string(),
                timestamp: Utc::now(),
            });
        }

        // Should only keep the 5 most recent
        assert_eq!(state.event_feed.len(), 5);

        // The newest event should be first
        assert_eq!(state.event_feed.front().unwrap().topic, "Topic 9");
        assert_eq!(state.event_feed.back().unwrap().topic, "Topic 5");
    }

    #[test]
    fn test_format_topic_name() {
        assert_eq!(format_topic_name(0), "Unspecified");
        assert_eq!(format_topic_name(1), "Boot Signal");
        assert_eq!(format_topic_name(11), "Subsystem Crashed");
        assert_eq!(format_topic_name(62), "Thought Surfaced");
        assert_eq!(format_topic_name(999), "Unknown Topic");
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        assert_eq!(truncate_with_ellipsis("short", 10), "short");
        assert_eq!(
            truncate_with_ellipsis("this is a very long string", 10),
            "this is..."
        );
        assert_eq!(truncate_with_ellipsis("exact", 5), "exact");
        assert_eq!(truncate_with_ellipsis("toolong", 5), "to...");
    }
}
