use std::collections::VecDeque;

use freya::prelude::*;

use crate::debug_state::{self, ThoughtEvent};
use crate::theme;

/// Renders the CTP thought stream — a scrollable list of surfaced thoughts,
/// newest at top, capped at `max_visible` entries.
pub fn thought_stream(
    thoughts: &VecDeque<ThoughtEvent>,
    max_visible: usize,
    empty_message: &str,
) -> impl IntoElement {
    let mut children: Vec<Element> = Vec::new();

    if thoughts.is_empty() {
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
        for (idx, thought) in thoughts.iter().enumerate() {
            if idx >= max_visible {
                break;
            }
            children.push(thought_row(thought).into());
        }
    }

    rect()
        .width(Size::fill())
        .child(
            label()
                .font_size(14.0)
                .color(theme::SECTION_HEADER_COLOR)
                .font_weight(FontWeight::BOLD)
                .text("Thought Stream"),
        )
        .children(children)
}

/// A single thought row: score badge + truncated content + timestamp.
fn thought_row(thought: &ThoughtEvent) -> Element {
    let badge_color = score_color(thought.relevance_score);
    let score_text = format!("{:.2}", thought.relevance_score);
    let content_text = debug_state::truncate_with_ellipsis(&thought.content, 80);
    let time_text = debug_state::format_relative_time(thought.timestamp);

    rect()
        .width(Size::fill())
        .direction(Direction::Horizontal)
        .cross_align(Alignment::Center)
        .padding(Gaps::new(2.0, 4.0, 2.0, 4.0))
        .child(
            // Score badge.
            rect()
                .background(badge_color)
                .corner_radius(4.0)
                .padding(Gaps::new(1.0, 6.0, 1.0, 6.0))
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_PRIMARY)
                        .text(score_text),
                ),
        )
        .child(
            rect().width(Size::px(8.0)), // spacer
        )
        .child(
            rect().width(Size::fill()).child(
                label()
                    .font_size(12.0)
                    .color(theme::TEXT_PRIMARY)
                    .text(content_text),
            ),
        )
        .child(
            label()
                .font_size(11.0)
                .color(theme::TEXT_MUTED)
                .text(time_text),
        )
        .into()
}

/// Map a relevance score to its badge colour (from theme).
pub fn score_color(score: f32) -> (u8, u8, u8) {
    if score > theme::SCORE_HIGH_THRESHOLD {
        theme::SCORE_HIGH_COLOR
    } else if score > theme::SCORE_MEDIUM_THRESHOLD {
        theme::SCORE_MEDIUM_COLOR
    } else {
        theme::SCORE_LOW_COLOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_thought_stream_renders_feed() {
        let mut thoughts = VecDeque::new();
        for i in 0..3 {
            thoughts.push_back(ThoughtEvent {
                content: format!("thought_{i}"),
                relevance_score: 0.5,
                timestamp: Utc::now(),
            });
        }
        let _element = thought_stream(&thoughts, 100, "No thoughts");
    }

    #[test]
    fn test_thought_stream_empty_state() {
        let thoughts = VecDeque::new();
        let _element = thought_stream(&thoughts, 100, "No thoughts surfaced yet");
    }

    #[test]
    fn test_thought_row_score_colour_high() {
        assert_eq!(score_color(0.9), theme::SCORE_HIGH_COLOR);
    }

    #[test]
    fn test_thought_row_score_colour_medium() {
        assert_eq!(score_color(0.65), theme::SCORE_MEDIUM_COLOR);
    }

    #[test]
    fn test_thought_row_score_colour_low() {
        assert_eq!(score_color(0.3), theme::SCORE_LOW_COLOR);
    }

    #[test]
    fn test_thought_stream_capped_at_100() {
        let mut thoughts = VecDeque::new();
        for i in 0..150 {
            thoughts.push_back(ThoughtEvent {
                content: format!("thought_{i}"),
                relevance_score: 0.5,
                timestamp: Utc::now(),
            });
        }
        // The component caps at max_visible; we verify the function doesn't panic
        // and the logic only iterates up to max_visible.
        let _element = thought_stream(&thoughts, 100, "No thoughts");
    }
}
