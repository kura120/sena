use chrono::{DateTime, Utc};
use freya::prelude::*;

use crate::debug_state;
use crate::theme;

/// Renders the system stats bar at the bottom of the debug panel.
/// Shows daemon-bus uptime, total events received, and gRPC connection status.
pub fn system_stats_bar(
    started_at: Option<DateTime<Utc>>,
    total_events: u64,
    connected: bool,
    connected_label: &str,
    reconnecting_label: &str,
    uptime_label: &str,
    events_label: &str,
) -> impl IntoElement {
    let uptime_text = debug_state::format_uptime(started_at);
    let events_text = total_events.to_string();

    let connection_color = if connected {
        theme::STATUS_READY_COLOR
    } else {
        theme::STATUS_UNAVAILABLE_COLOR
    };
    let connection_text = if connected {
        connected_label
    } else {
        reconnecting_label
    };

    rect()
        .width(Size::fill())
        // Top border line.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(1.0))
                .background(theme::STATS_BAR_BORDER),
        )
        // Stats content row.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(31.0))
                .background(theme::STATS_BAR_BACKGROUND)
                .direction(Direction::Horizontal)
                .cross_align(Alignment::Center)
                .padding(Gaps::new(4.0, 12.0, 4.0, 12.0))
                // Uptime.
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_SECONDARY)
                        .text(format!("{uptime_label}: ")),
                )
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_PRIMARY)
                        .text(uptime_text),
                )
                .child(
                    rect().width(Size::px(20.0)), // spacer
                )
                // Total events.
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_SECONDARY)
                        .text(format!("{events_label}: ")),
                )
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_PRIMARY)
                        .text(events_text),
                )
                .child(
                    rect().width(Size::fill()), // spacer
                )
                // Connection status.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_bar_connected() {
        let _element = system_stats_bar(
            Some(Utc::now()),
            42,
            true,
            "Connected",
            "Reconnecting",
            "Uptime",
            "Events",
        );
    }

    #[test]
    fn test_stats_bar_disconnected() {
        let _element = system_stats_bar(
            None,
            0,
            false,
            "Connected",
            "Reconnecting",
            "Uptime",
            "Events",
        );
    }

    #[test]
    fn test_stats_bar_connection_colors_differ() {
        // Connected uses green, disconnected uses red.
        assert_ne!(theme::STATUS_READY_COLOR, theme::STATUS_UNAVAILABLE_COLOR);
    }
}
