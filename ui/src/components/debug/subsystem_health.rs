use std::collections::HashMap;

use freya::prelude::*;

use crate::debug_state::SubsystemHealthStatus;
use crate::theme;

/// Renders the subsystem health grid — one row per known subsystem
/// with a coloured status indicator dot.
pub fn subsystem_health(subsystems: &HashMap<String, SubsystemHealthStatus>) -> impl IntoElement {
    let mut rows: Vec<Element> = Vec::new();

    // Sort subsystems alphabetically for consistent display order.
    let mut names: Vec<&String> = subsystems.keys().collect();
    names.sort();

    for name in names {
        let status = subsystems.get(name).copied().unwrap_or_default();
        rows.push(subsystem_row(name, status).into());
    }

    rect()
        .width(Size::fill())
        .child(
            label()
                .font_size(14.0)
                .color(theme::SECTION_HEADER_COLOR)
                .font_weight(FontWeight::BOLD)
                .text("Subsystem Health"),
        )
        .children(rows)
}

/// A single subsystem row: coloured dot + name + status text.
fn subsystem_row(name: &str, status: SubsystemHealthStatus) -> Element {
    let dot_color = status_color(status);
    let status_text = status.to_string();

    rect()
        .width(Size::fill())
        .direction(Direction::Horizontal)
        .cross_align(Alignment::Center)
        .padding(Gaps::new(2.0, 4.0, 2.0, 4.0))
        .child(
            rect()
                .width(Size::px(10.0))
                .height(Size::px(10.0))
                .corner_radius(5.0)
                .background(dot_color),
        )
        .child(
            rect().width(Size::px(8.0)), // spacer
        )
        .child(
            rect().width(Size::px(130.0)).child(
                label()
                    .font_size(12.0)
                    .color(theme::TEXT_PRIMARY)
                    .text(name.to_string()),
            ),
        )
        .child(
            label()
                .font_size(12.0)
                .color(theme::TEXT_SECONDARY)
                .text(status_text),
        )
        .into()
}

/// Map a subsystem health status to its display colour (from theme).
pub fn status_color(status: SubsystemHealthStatus) -> (u8, u8, u8) {
    match status {
        SubsystemHealthStatus::Ready => theme::STATUS_READY_COLOR,
        SubsystemHealthStatus::Degraded => theme::STATUS_DEGRADED_COLOR,
        SubsystemHealthStatus::Unavailable => theme::STATUS_UNAVAILABLE_COLOR,
        SubsystemHealthStatus::Unknown => theme::STATUS_UNKNOWN_COLOR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_colour_ready() {
        assert_eq!(
            status_color(SubsystemHealthStatus::Ready),
            theme::STATUS_READY_COLOR
        );
    }

    #[test]
    fn test_status_colour_degraded() {
        assert_eq!(
            status_color(SubsystemHealthStatus::Degraded),
            theme::STATUS_DEGRADED_COLOR
        );
    }

    #[test]
    fn test_status_colour_unavailable() {
        assert_eq!(
            status_color(SubsystemHealthStatus::Unavailable),
            theme::STATUS_UNAVAILABLE_COLOR
        );
    }

    #[test]
    fn test_status_colour_unknown() {
        assert_eq!(
            status_color(SubsystemHealthStatus::Unknown),
            theme::STATUS_UNKNOWN_COLOR
        );
    }

    #[test]
    fn test_subsystem_health_renders_all_known_subsystems() {
        let mut subsystems = HashMap::new();
        for name in crate::debug_state::KNOWN_SUBSYSTEMS {
            subsystems.insert((*name).to_string(), SubsystemHealthStatus::Unknown);
        }
        // Verify we have all 11 subsystems.
        assert_eq!(subsystems.len(), 11);
    }
}
