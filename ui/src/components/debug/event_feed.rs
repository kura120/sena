use std::collections::VecDeque;

use freya::prelude::*;

use crate::debug_state::{self, BusEventEntry};
use crate::theme;

/// Renders the event bus monitor — a scrollable list of all daemon-bus events,
/// newest at top, capped at `max_visible` entries.
pub fn event_feed(
    events: &VecDeque<BusEventEntry>,
    max_visible: usize,
    empty_message: &str,
) -> impl IntoElement {
    let mut children: Vec<Element> = Vec::new();

    if events.is_empty() {
        children.push(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(8.0, 4.0, 8.0, 4.0))
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text(empty_message.to_string()),
                )
                .into(),
        );
    } else {
        for (idx, event) in events.iter().enumerate() {
            if idx >= max_visible {
                break;
            }
            children.push(event_row(event).into());
        }
    }

    rect()
        .width(Size::fill())
        .child(
            label()
                .font_size(14.0)
                .color(theme::SECTION_HEADER_COLOR)
                .font_weight(FontWeight::BOLD)
                .text("Event Bus"),
        )
        .children(children)
}

/// A single event row: topic name + source + truncated payload + timestamp.
fn event_row(event: &BusEventEntry) -> Element {
    let time_text = debug_state::format_relative_time(event.timestamp);
    let payload_text = debug_state::truncate_with_ellipsis(&event.payload_summary, 60);

    rect()
        .width(Size::fill())
        .padding(Gaps::new(2.0, 4.0, 2.0, 4.0))
        .child(
            rect()
                .width(Size::fill())
                .direction(Direction::Horizontal)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_PRIMARY)
                        .font_weight(FontWeight::BOLD)
                        .text(event.topic.clone()),
                )
                .child(
                    rect().width(Size::fill()), // spacer
                )
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_MUTED)
                        .text(time_text),
                ),
        )
        .child(
            label()
                .font_size(11.0)
                .color(theme::TEXT_SECONDARY)
                .text(payload_text),
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_event_feed_renders_events() {
        let mut events = VecDeque::new();
        for i in 0..5 {
            events.push_back(BusEventEntry {
                topic: format!("Topic {i}"),
                source_subsystem: "test".to_string(),
                payload_summary: format!("payload_{i}"),
                timestamp: Utc::now(),
            });
        }
        let _element = event_feed(&events, 200, "No events");
    }

    #[test]
    fn test_event_feed_empty_state() {
        let events = VecDeque::new();
        let _element = event_feed(&events, 200, "No events yet");
    }

    #[test]
    fn test_event_row_topic_name_formatted() {
        // Verify format_topic_name returns human-readable strings, not raw debug.
        let name = debug_state::format_topic_name(62);
        assert_eq!(name, "Thought Surfaced");
        assert!(
            !name.contains("TOPIC_"),
            "topic name should be human-readable, got: {name}"
        );
    }

    #[test]
    fn test_event_feed_capped_at_200() {
        let mut events = VecDeque::new();
        for i in 0..300 {
            events.push_back(BusEventEntry {
                topic: format!("Topic {i}"),
                source_subsystem: "test".to_string(),
                payload_summary: String::new(),
                timestamp: Utc::now(),
            });
        }
        // The component caps at max_visible; verify no panic.
        let _element = event_feed(&events, 200, "No events");
    }

    #[test]
    fn test_event_row_payload_truncated() {
        let long_payload = "a".repeat(100);
        let truncated = debug_state::truncate_with_ellipsis(&long_payload, 60);
        assert!(truncated.len() <= 63); // 59 + multibyte ellipsis
        assert!(truncated.ends_with('…'));
    }
}
