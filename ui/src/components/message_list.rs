use freya::prelude::*;

use crate::state::ChatMessage;
use crate::theme;

/// Renders the conversation history as a scrollable list of chat bubbles.
pub fn message_list(messages: Vec<ChatMessage>) -> impl IntoElement {
    let mut children: Vec<Element> = Vec::new();

    for msg in &messages {
        let bubble_bg = if msg.is_user {
            (26_u8, 54_u8, 93_u8)
        } else {
            (45_u8, 27_u8, 78_u8)
        };
        children.push(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(6.0, 12.0, 6.0, 12.0))
                .child(
                    rect()
                        .background(bubble_bg)
                        .padding(Gaps::new(8.0, 12.0, 8.0, 12.0))
                        .child(
                            label()
                                .font_size(14.0)
                                .color(theme::TEXT_PRIMARY)
                                .text(msg.content.clone()),
                        ),
                )
                .into(),
        );
    }

    ScrollView::new()
        .width(Size::fill())
        .height(Size::fill())
        .children(children)
}