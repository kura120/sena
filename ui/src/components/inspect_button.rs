use freya::prelude::*;

use crate::theme;

/// Renders the "Inspect" button in the main window header.
/// Receives `label_text` as a prop — no hardcoded strings.
/// Calls `on_toggle` when pressed.
pub fn inspect_button(label_text: &str, on_toggle: impl FnMut() + Clone + 'static) -> impl IntoElement {
    let on_toggle = on_toggle.clone();
    Button::new()
        .child(
            label()
                .color(theme::TEXT_PRIMARY)
                .font_size(13.0)
                .text(label_text.to_string()),
        )
        .on_press(move |_| {
            let mut cb = on_toggle.clone();
            cb();
        })
}
