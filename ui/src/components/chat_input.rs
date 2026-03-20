use freya::prelude::*;
use crate::theme::SENA_THEME;

#[allow(non_snake_case)]
#[component]
pub fn ChatInput(on_submit: EventHandler<String>) -> Element {
    let mut value = use_signal(|| String::new());

    let onkeydown = move |e: KeyboardEvent| {
        if e.key == Key::Enter {
            let text = value.read().clone();
            if !text.trim().is_empty() {
                on_submit.call(text);
                value.set(String::new());
            }
        }
    };

    rsx!(
        rect {
            padding: "10",
            background: "{SENA_THEME.input_background}",
            width: "100%",
            height: "60",
            corner_radius: "8",
            Input {
                value: value.read().clone(),
                onchange: move |e| value.set(e),
                onkeydown: onkeydown,
                theme: theme_with!(InputTheme {
                    background: "transparent".into(),
                    font_theme: theme_with!(FontTheme {
                        color: "white".into(),
                    }),
                })
            }
        }
    )
}