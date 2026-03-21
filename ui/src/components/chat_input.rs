use freya::prelude::*;

use crate::theme;

/// Chat input row — shows a hint label and a Send button.
/// Full text-input integration is deferred until freya-elements TextInput
/// is wired in; the state structures and service contracts are ready.
pub fn chat_input(on_submit: impl Fn(String) + Clone + 'static) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::px(60.0))
        .background(theme::APP_BACKGROUND)
        .padding(Gaps::new(8.0, 12.0, 8.0, 12.0))
        .direction(Direction::Horizontal)
        .cross_align(Alignment::Center)
        .child(
            label()
                .font_size(14.0)
                .color(theme::TEXT_MUTED)
                .text("Chat ready — reactive-loop integration pending"),
        )
        .child(rect().width(Size::fill()))
        .child(
            Button::new()
                .on_press(move |_| {
                    on_submit(String::new());
                })
                .child(
                    label()
                        .font_size(13.0)
                        .color(theme::TEXT_PRIMARY)
                        .text("Send"),
                ),
        )
}