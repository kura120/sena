#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod components;
mod config;
mod state;
mod theme;

use freya::prelude::*;
use crate::config::UiConfig;
use crate::components::chat_input::ChatInput;
use crate::components::message_list::MessageList;
use crate::components::debug_panel::DebugPanel;
use crate::state::ChatMessage;

fn main() {
    let config = UiConfig::load();
    
    launch_cfg(
        app,
        LaunchConfig::builder()
            .with_title(&config.window.title)
            .with_width(config.window.width)
            .with_height(config.window.height)
            .with_min_width(config.window.min_width)
            .with_min_height(config.window.min_height)
            .build(),
    );
}

fn app() -> Element {
    let mut messages = use_signal::<Vec<ChatMessage>>(|| Vec::new());
    let mut show_debug = use_signal(|| false);

    let on_submit = move |text: String| {
        messages.write().push(ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            content: text,
            is_user: true,
            timestamp: chrono::Local::now().to_string(),
            latency_ms: None,
            assembly_trace: None,
        });
    };
    
    // Global keyboard listener for F12
    let onglobaldown = move |e: KeyboardEvent| {
        if e.key == Key::F12 {
            show_debug.toggle();
        }
    };

    rsx!(
        rect {
            width: "100%",
            height: "100%",
            background: "{theme::SENA_THEME.background}",
            color: "{theme::SENA_THEME.on_background}",
            onkeydown: onglobaldown,
            
            rect {
                width: "100%",
                height: "calc(100% - 70)", // Subtract input height
                MessageList { messages: messages.read().clone() }
            }
            
            rect {
                width: "100%",
                height: "70",
                padding: "10",
                ChatInput { on_submit: on_submit }
            }
            
            DebugPanel { visible: *show_debug.read() }
        }
    )
}
