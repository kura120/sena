use freya::prelude::*;

use crate::debug_state::DebugState;
use crate::theme;

use super::debug::event_feed::event_feed;
use super::debug::memory_stats::memory_stats_panel;
use super::debug::subsystem_health::subsystem_health;
use super::debug::thought_stream::thought_stream;
use super::debug::vram_bar::vram_bar;

/// Section separator line inside the debug panel.
fn section_separator() -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::px(1.0))
        .background(theme::PANEL_BORDER)
        .margin(Gaps::new(6.0, 0.0, 6.0, 0.0))
}

/// Renders the debug panel overlay.
/// When `open` is false, renders nothing (zero-size element).
/// When `open` is true, renders the full debug panel content.
pub fn debug_panel(
    open: bool,
    panel_width: f64,
    state: &DebugState,
    panel_title: &str,
    thought_empty_message: &str,
    event_empty_message: &str,
    connection_status_label: &str,
) -> Element {
    if !open {
        return Element::from(rect().width(Size::px(0.0)).height(Size::px(0.0)));
    }

    let connection_text = if state.connected {
        "Connected"
    } else {
        connection_status_label
    };

    let connection_color = if state.connected {
        theme::STATUS_READY_COLOR
    } else {
        theme::STATUS_UNAVAILABLE_COLOR
    };

    rect()
        .height(Size::fill())
        .width(Size::px(panel_width as f32))
        .background(theme::PANEL_BACKGROUND)
        .overflow(Overflow::Clip)
        // Panel header.
        .child(
            rect()
                .width(Size::fill())
                .background(theme::PANEL_HEADER_BACKGROUND)
                .padding(Gaps::new(8.0, 12.0, 8.0, 12.0))
                .direction(Direction::Horizontal)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .font_size(16.0)
                        .color(theme::TEXT_PRIMARY)
                        .font_weight(FontWeight::BOLD)
                        .text(panel_title.to_string()),
                )
                .child(
                    rect().width(Size::fill()), // spacer
                )
                .child(
                    rect()
                        .width(Size::px(8.0))
                        .height(Size::px(8.0))
                        .corner_radius(4.0)
                        .background(connection_color),
                )
                .child(
                    rect().width(Size::px(6.0)), // spacer
                )
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_SECONDARY)
                        .text(connection_text.to_string()),
                ),
        )
        // Scrollable content area.
        .child(
            ScrollView::new()
                .width(Size::fill())
                .height(Size::fill())
                .child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new(8.0, 12.0, 8.0, 12.0))
                        // Subsystem health.
                        .child(subsystem_health(&state.subsystem_health))
                        .child(section_separator())
                        // VRAM usage.
                        .child(vram_bar(state.vram_used_mb, state.vram_total_mb))
                        .child(section_separator())
                        // Memory stats.
                        .child(memory_stats_panel(&state.memory_stats))
                        .child(section_separator())
                        // Thought stream.
                        .child(thought_stream(
                            &state.thought_feed,
                            state.thought_feed_max,
                            thought_empty_message,
                        ))
                        .child(section_separator())
                        // Event bus feed.
                        .child(event_feed(
                            &state.event_feed,
                            state.event_feed_max,
                            event_empty_message,
                        )),
                ),
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_panel_hidden_when_closed() {
        let state = DebugState::default();
        let _element = debug_panel(
            false, 420.0, &state, "Debug", "No thoughts", "No events", "Offline",
        );
    }

    #[test]
    fn test_debug_panel_visible_when_open() {
        let state = DebugState::default();
        let _element = debug_panel(
            true, 420.0, &state, "Debug", "No thoughts", "No events", "Offline",
        );
    }
}
