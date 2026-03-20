use freya::prelude::*;

use crate::debug_state::DebugState;
use crate::theme;

use super::debug::boot_signal_history::boot_signal_history;
use super::debug::event_feed::event_feed;
use super::debug::subsystem_health::subsystem_health;
use super::debug::system_stats_bar::system_stats_bar;

/// Section separator line inside panels.
fn section_separator() -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::px(1.0))
        .background(theme::SECTION_DIVIDER)
}

/// Renders the debug panel.
/// When `open` is false, renders nothing (zero-size element).
/// When `open` is true, renders the full 3-zone layout:
///   - Left sidebar (~280px): Subsystem Status Panel
///   - Main area: Event Bus Stream (top 60%) + Boot Signal History (bottom 40%)
///   - Bottom bar: System Stats
pub fn debug_panel(
    open: bool,
    state: &DebugState,
    panel_title: &str,
    event_empty_message: &str,
    boot_empty_message: &str,
    connection_status_label: &str,
    boot_history_title: &str,
    uptime_label: &str,
    events_label: &str,
    connected_label: &str,
    required_badge_label: &str,
    optional_badge_label: &str,
) -> Element {
    if !open {
        return Element::from(rect().width(Size::px(0.0)).height(Size::px(0.0)));
    }

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .overflow(Overflow::Clip)
        .background(theme::PANEL_BACKGROUND)
        .child(
            // Panel header bar.
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
                ),
        )
        .child(
            // Main body: left sidebar + main content area.
            rect()
                .width(Size::fill())
                .height(Size::fill())
                .overflow(Overflow::Clip)
                .direction(Direction::Horizontal)
                // Left sidebar — subsystem health.
                .child(
                    rect()
                        .width(Size::px(280.0))
                        .min_width(Size::px(280.0))
                        .height(Size::fill())
                        .background(theme::SIDEBAR_BACKGROUND)
                        .child(
                            ScrollView::new()
                                .width(Size::fill())
                                .height(Size::fill())
                                .child(
                                    rect()
                                        .width(Size::fill())
                                        .padding(Gaps::new(8.0, 12.0, 8.0, 12.0))
                                        .child(subsystem_health(&state.subsystem_health)),
                                ),
                        ),
                )
                // Vertical separator between sidebar and main area.
                .child(
                    rect()
                        .width(Size::px(1.0))
                        .height(Size::fill())
                        .background(theme::SECTION_DIVIDER),
                )
                // Main content area — event stream + boot history.
                .child(
                    rect()
                        .width(Size::fill())
                        .height(Size::fill())
                        .overflow(Overflow::Clip)
                        // Top 60%: Event Bus Stream.
                        .child(
                            rect()
                                .width(Size::fill())
                                .height(Size::percent(60.0))
                                .overflow(Overflow::Clip)
                                .child(
                                    ScrollView::new()
                                        .width(Size::fill())
                                        .height(Size::fill())
                                        .child(
                                            rect()
                                                .width(Size::fill())
                                                .padding(Gaps::new(8.0, 12.0, 8.0, 12.0))
                                                .child(event_feed(
                                                    &state.event_feed,
                                                    state.event_feed_max,
                                                    event_empty_message,
                                                )),
                                        ),
                                ),
                        )
                        // Horizontal separator.
                        .child(section_separator())
                        // Bottom 40%: Boot Signal History.
                        .child(
                            rect()
                                .width(Size::fill())
                                .height(Size::percent(40.0))
                                .overflow(Overflow::Clip)
                                .child(boot_signal_history(
                                    &state.boot_signal_history,
                                    boot_empty_message,
                                    boot_history_title,
                                    required_badge_label,
                                    optional_badge_label,
                                )),
                        ),
                ),
        )
        // Bottom stats bar.
        .child(system_stats_bar(
            state.started_at,
            state.total_events_received,
            state.connected,
            connected_label,
            connection_status_label,
            uptime_label,
            events_label,
        ))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_panel_hidden_when_closed() {
        let state = DebugState::default();
        let _element = debug_panel(
            false,
            &state,
            "Debug Panel",
            "No events",
            "No boot signals",
            "Reconnecting",
            "Boot Signals",
            "Uptime",
            "Events",
            "Connected",
            "REQ",
            "OPT",
        );
    }

    #[test]
    fn test_debug_panel_visible_when_open() {
        let state = DebugState::default();
        let _element = debug_panel(
            true,
            &state,
            "Debug Panel",
            "No events",
            "No boot signals",
            "Reconnecting",
            "Boot Signals",
            "Uptime",
            "Events",
            "Connected",
            "REQ",
            "OPT",
        );
    }
}
