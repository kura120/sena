use freya::prelude::*;

use crate::theme;

/// Renders a VRAM usage progress bar with colour thresholds.
pub fn vram_bar(used_mb: u32, total_mb: u32) -> impl IntoElement {
    let (percentage, bar_color) = vram_percentage_and_color(used_mb, total_mb);
    let fill_pct = percentage * 100.0;
    let usage_text = format!("{used_mb} / {total_mb} MB");
    let pct_text = format!("{fill_pct:.0}%");

    rect()
        .width(Size::fill())
        .child(
            label()
                .font_size(14.0)
                .color(theme::SECTION_HEADER_COLOR)
                .font_weight(FontWeight::BOLD)
                .text("VRAM Usage"),
        )
        .child(
            rect()
                .width(Size::fill())
                .direction(Direction::Horizontal)
                .cross_align(Alignment::Center)
                .padding(Gaps::new(4.0, 0.0, 2.0, 0.0))
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_PRIMARY)
                        .text(usage_text),
                )
                .child(
                    rect().width(Size::fill()), // spacer
                )
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_SECONDARY)
                        .text(pct_text),
                ),
        )
        .child(
            // Bar track.
            rect()
                .width(Size::fill())
                .height(Size::px(8.0))
                .corner_radius(4.0)
                .background(theme::VRAM_BAR_TRACK)
                .child(
                    // Bar fill.
                    rect()
                        .width(Size::percent(fill_pct))
                        .height(Size::px(8.0))
                        .corner_radius(4.0)
                        .background(bar_color),
                ),
        )
}

/// Compute the fill percentage (clamped 0.0–1.0) and the appropriate colour.
pub fn vram_percentage_and_color(used_mb: u32, total_mb: u32) -> (f32, (u8, u8, u8)) {
    if total_mb == 0 {
        return (0.0, theme::VRAM_NORMAL_COLOR);
    }
    let percentage = (used_mb as f32 / total_mb as f32).clamp(0.0, 1.0);
    let color = if percentage > theme::VRAM_CRITICAL_THRESHOLD {
        theme::VRAM_CRITICAL_COLOR
    } else if percentage > theme::VRAM_WARNING_THRESHOLD {
        theme::VRAM_WARNING_COLOR
    } else {
        theme::VRAM_NORMAL_COLOR
    };
    (percentage, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vram_bar_percentage_calculation() {
        let (pct, _) = vram_percentage_and_color(2048, 4096);
        assert!((pct - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_vram_bar_percentage_clamped() {
        let (pct, _) = vram_percentage_and_color(5000, 4096);
        assert!((pct - 1.0).abs() < f32::EPSILON, "should clamp at 1.0");
    }

    #[test]
    fn test_vram_bar_colour_normal() {
        let (_, color) = vram_percentage_and_color(1000, 4096); // ~24%
        assert_eq!(color, theme::VRAM_NORMAL_COLOR);
    }

    #[test]
    fn test_vram_bar_colour_warning() {
        let (_, color) = vram_percentage_and_color(3500, 4096); // ~85%
        assert_eq!(color, theme::VRAM_WARNING_COLOR);
    }

    #[test]
    fn test_vram_bar_colour_critical() {
        let (_, color) = vram_percentage_and_color(3950, 4096); // ~96%
        assert_eq!(color, theme::VRAM_CRITICAL_COLOR);
    }

    #[test]
    fn test_vram_bar_zero_total() {
        let (pct, color) = vram_percentage_and_color(0, 0);
        assert!((pct - 0.0).abs() < f32::EPSILON);
        assert_eq!(color, theme::VRAM_NORMAL_COLOR);
    }
}
