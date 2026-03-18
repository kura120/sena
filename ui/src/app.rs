use std::sync::Arc;

use freya::prelude::*;
use tokio::sync::RwLock;

use crate::components::debug_panel::debug_panel;
use crate::components::inspect_button::inspect_button;
use crate::debug_state::DebugState;
use crate::theme;

/// Root application component.
/// Provides the top-level layout with header, main content area, and debug panel overlay.
pub fn app(
    debug_state_handle: Arc<RwLock<DebugState>>,
    panel_width: f64,
    panel_title: &'static str,
    inspect_label: &'static str,
    thought_empty_message: &'static str,
    event_empty_message: &'static str,
    connection_status_label: &'static str,
) -> impl IntoElement {
    let mut panel_open = use_state(|| false);

    // Read a snapshot of DebugState for rendering.
    // Use a side effect to periodically refresh the snapshot from the Arc<RwLock<>>.
    let debug_state_for_effect = debug_state_handle.clone();
    let debug_snapshot = use_state(|| DebugState::default());

    use_side_effect(move || {
        let handle = debug_state_for_effect.clone();
        let mut writer = debug_snapshot;
        spawn(async move {
            loop {
                {
                    let state = handle.read().await;
                    writer.set(state.clone());
                }
                // Refresh at ~4 Hz to stay responsive without thrashing.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        });
    });

    let is_open = *panel_open.read();
    let snapshot = debug_snapshot.read();

    let toggle = move || {
        let current = *panel_open.read();
        panel_open.set(!current);
    };

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::APP_BACKGROUND)
        .on_key_up(move |event: Event<KeyboardEventData>| {
            if event.key == Key::Named(NamedKey::F12) {
                let current = *panel_open.read();
                panel_open.set(!current);
            }
        })
        // Header bar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(40.0))
                .background(theme::HEADER_BACKGROUND)
                .direction(Direction::Horizontal)
                .cross_align(Alignment::Center)
                .padding(Gaps::new(4.0, 12.0, 4.0, 12.0))
                .child(
                    label()
                        .font_size(16.0)
                        .color(theme::TEXT_PRIMARY)
                        .font_weight(FontWeight::BOLD)
                        .text("Sena"),
                )
                .child(
                    rect().width(Size::fill()), // spacer
                )
                .child(inspect_button(inspect_label, toggle)),
        )
        // Body: main content + optional debug panel.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::fill())
                .direction(Direction::Horizontal)
                // Main content area.
                .child(
                    rect()
                        .width(Size::fill())
                        .height(Size::fill())
                        .child(
                            rect()
                                .width(Size::fill())
                                .height(Size::fill())
                                .main_align(Alignment::Center)
                                .cross_align(Alignment::Center)
                                .child(
                                    label()
                                        .font_size(14.0)
                                        .color(theme::TEXT_MUTED)
                                        .text("Main content area"),
                                ),
                        ),
                )
                // Debug panel overlay.
                .child(debug_panel(
                    is_open,
                    panel_width,
                    &snapshot,
                    panel_title,
                    thought_empty_message,
                    event_empty_message,
                    connection_status_label,
                )),
        )
}
