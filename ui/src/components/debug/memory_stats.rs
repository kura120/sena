use freya::prelude::*;

use crate::debug_state::{self, MemoryStats as MemoryStatsData};
use crate::theme;

/// Renders memory tier statistics: entry counts and last write timestamp.
pub fn memory_stats_panel(stats: &MemoryStatsData) -> impl IntoElement {
    let last_write_text = match stats.last_write {
        Some(ts) => debug_state::format_relative_time(ts),
        None => "never".to_string(),
    };

    rect()
        .width(Size::fill())
        .child(
            label()
                .font_size(14.0)
                .color(theme::SECTION_HEADER_COLOR)
                .font_weight(FontWeight::BOLD)
                .text("Memory Tiers"),
        )
        .child(tier_row("Short-term", stats.short_term_count))
        .child(tier_row("Long-term", stats.long_term_count))
        .child(tier_row("Episodic", stats.episodic_count))
        .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(4.0, 4.0, 2.0, 4.0))
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_MUTED)
                        .text(format!("Last write: {last_write_text}")),
                ),
        )
}

/// A single tier row: name + entry count.
fn tier_row(tier_name: &str, count: u32) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .direction(Direction::Horizontal)
        .padding(Gaps::new(2.0, 4.0, 2.0, 4.0))
        .child(
            rect().width(Size::px(100.0)).child(
                label()
                    .font_size(12.0)
                    .color(theme::TEXT_PRIMARY)
                    .text(tier_name.to_string()),
            ),
        )
        .child(
            label()
                .font_size(12.0)
                .color(theme::TEXT_SECONDARY)
                .text(count.to_string()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug_state::MemoryStats as MemoryStatsData;

    #[test]
    fn test_memory_stats_renders_all_three_tiers() {
        let stats = MemoryStatsData {
            short_term_count: 5,
            long_term_count: 10,
            episodic_count: 3,
            last_write: None,
        };
        // Should not panic and constructs the component.
        let _element = memory_stats_panel(&stats);
    }

    #[test]
    fn test_memory_stats_last_write_formats_correctly() {
        let stats = MemoryStatsData {
            short_term_count: 0,
            long_term_count: 0,
            episodic_count: 0,
            last_write: Some(chrono::Utc::now() - chrono::Duration::seconds(2)),
        };
        let time_str = debug_state::format_relative_time(stats.last_write.expect("should have timestamp"));
        assert!(
            time_str.contains("s ago") || time_str == "just now",
            "expected relative time, got: {time_str}"
        );
    }
}
