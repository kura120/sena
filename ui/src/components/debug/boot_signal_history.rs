use std::collections::VecDeque;

use freya::prelude::*;

use crate::debug_state::{self, BootSignalEntry};
use crate::theme;

/// Renders the boot signal history — a scrollable list of boot signals received,
/// newest at top.
pub fn boot_signal_history(
    signals: &VecDeque<BootSignalEntry>,
    empty_message: &str,
    section_title: &str,
    required_badge_label: &str,
    optional_badge_label: &str,
) -> impl IntoElement {
    let mut children: Vec<Element> = Vec::new();

    if signals.is_empty() {
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
        let req_label = required_badge_label.to_string();
        let opt_label = optional_badge_label.to_string();
        for signal in signals.iter() {
            children.push(boot_signal_row(signal, &req_label, &opt_label).into());
        }
    }

    rect()
        .width(Size::fill())
        .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(8.0, 12.0, 4.0, 12.0))
                .child(
                    label()
                        .font_size(14.0)
                        .color(theme::SECTION_HEADER_COLOR)
                        .font_weight(FontWeight::BOLD)
                        .text(section_title.to_string()),
                ),
        )
        .child(
            ScrollView::new()
                .width(Size::fill())
                .height(Size::fill())
                .child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new(0.0, 12.0, 8.0, 12.0))
                        .children(children),
                ),
        )
}

/// A single boot signal row: required/optional badge + signal name + source + timestamp.
fn boot_signal_row(signal: &BootSignalEntry, required_label: &str, optional_label: &str) -> Element {
    let badge_color = if signal.required {
        theme::BOOT_REQUIRED_COLOR
    } else {
        theme::BOOT_OPTIONAL_COLOR
    };
    let badge_text = if signal.required {
        required_label
    } else {
        optional_label
    };
    let time_text = debug_state::format_relative_time(signal.timestamp);

    rect()
        .width(Size::fill())
        .overflow(Overflow::Clip)
        .direction(Direction::Horizontal)
        .cross_align(Alignment::Center)
        .padding(Gaps::new(2.0, 4.0, 2.0, 4.0))
        // Required/optional badge.
        .child(
            rect()
                .background(badge_color)
                .corner_radius(3.0)
                .padding(Gaps::new(1.0, 5.0, 1.0, 5.0))
                .child(
                    label()
                        .font_size(9.0)
                        .color(theme::TEXT_PRIMARY)
                        .font_weight(FontWeight::BOLD)
                        .text(badge_text.to_string()),
                ),
        )
        .child(
            rect().width(Size::px(6.0)), // spacer
        )
        // Signal name — takes remaining space between badge and metadata.
        .child(
            rect().width(Size::fill()).overflow(Overflow::Clip).child(
                label()
                    .font_size(12.0)
                    .color(theme::TEXT_PRIMARY)
                    .text(signal.signal_name.clone()),
            ),
        )
        // Source subsystem.
        .child(
            rect().width(Size::px(100.0)).child(
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_SECONDARY)
                    .text(signal.source_subsystem.clone()),
            ),
        )
        .child(
            rect().width(Size::px(8.0)), // spacer
        )
        // Timestamp.
        .child(
            rect().width(Size::px(60.0)).child(
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_MUTED)
                    .text(time_text),
            ),
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_boot_signal_history_empty_state() {
        let signals = VecDeque::new();
        let _element = boot_signal_history(&signals, "No boot signals", "Boot Signals", "REQ", "OPT");
    }

    #[test]
    fn test_boot_signal_history_renders_entries() {
        let mut signals = VecDeque::new();
        signals.push_back(BootSignalEntry {
            signal_name: "DAEMON_BUS_READY".to_string(),
            source_subsystem: "daemon-bus".to_string(),
            required: true,
            timestamp: Utc::now(),
        });
        signals.push_back(BootSignalEntry {
            signal_name: "LORA_SKIPPED".to_string(),
            source_subsystem: "lora-manager".to_string(),
            required: false,
            timestamp: Utc::now(),
        });
        let _element = boot_signal_history(&signals, "No boot signals", "Boot Signals", "REQ", "OPT");
    }

    #[test]
    fn test_boot_signal_required_badge_color() {
        let required = BootSignalEntry {
            signal_name: "TEST".to_string(),
            source_subsystem: "test".to_string(),
            required: true,
            timestamp: Utc::now(),
        };
        let optional = BootSignalEntry {
            signal_name: "TEST".to_string(),
            source_subsystem: "test".to_string(),
            required: false,
            timestamp: Utc::now(),
        };
        // Verify badge colors differ between required and optional.
        assert_ne!(
            if required.required { theme::BOOT_REQUIRED_COLOR } else { theme::BOOT_OPTIONAL_COLOR },
            if optional.required { theme::BOOT_REQUIRED_COLOR } else { theme::BOOT_OPTIONAL_COLOR },
        );
    }
}
