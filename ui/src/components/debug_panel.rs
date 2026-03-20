use freya::prelude::*;
use crate::theme::SENA_THEME;

#[allow(non_snake_case)]
#[component]
pub fn DebugPanel(visible: bool) -> Element {
    if !visible {
        return rsx!();
    }

    rsx!(
        rect {
            position: "absolute",
            top: "0",
            right: "0",
            width: "300",
            height: "100%",
            background: "{SENA_THEME.debug_background}",
            padding: "15",
            label {
                color: "{SENA_THEME.debug_text}",
                font_size: "18",
                "Debug Panel"
            }
            label {
                color: "{SENA_THEME.debug_text}",
                "Context Usage: 0/4096"
            }
        }
    )
}