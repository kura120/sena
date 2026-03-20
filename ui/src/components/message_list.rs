use freya::prelude::*;
use crate::state::ChatMessage;
use crate::theme::SENA_THEME;

#[allow(non_snake_case)]
#[component]
pub fn MessageList(messages: Vec<ChatMessage>) -> Element {
    rsx!(
        ScrollView {
            theme: theme_with!(ScrollViewTheme {
                width: "100%".into(),
                height: "100%".into(),
            }),
            for msg in messages {
                rect {
                    width: "100%",
                    padding: "10",
                    direction: "horizontal",
                    main_align: if msg.is_user { "end" } else { "start" },
                    rect {
                        background: if msg.is_user { SENA_THEME.user_message_bg } else { SENA_THEME.sena_message_bg },
                        corner_radius: "12",
                        padding: "12",
                        max_width: "80%",
                        label {
                            color: "white",
                            "{msg.content}"
                        }
                    }
                }
            }
        }
    )
}