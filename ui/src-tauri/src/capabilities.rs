//! Capability constants for the UI subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what features this subsystem provides.

/// Overlay window rendering and presentation
pub const OVERLAY_RENDERING: &str = "overlay_rendering";

/// System tray integration
pub const SYSTEM_TRAY: &str = "system_tray";

/// Global hotkey registration and handling
pub const GLOBAL_HOTKEYS: &str = "global_hotkeys";

/// Toast notification display
pub const TOAST_NOTIFICATIONS: &str = "toast_notifications";

/// Debug panel UI
pub const DEBUG_PANEL: &str = "debug_panel";

/// Returns the list of capabilities the UI subsystem currently provides.
///
/// This is called when signaling UI_READY to daemon-bus.
pub fn get_capabilities() -> Vec<String> {
    vec![
        OVERLAY_RENDERING.to_string(),
        SYSTEM_TRAY.to_string(),
        GLOBAL_HOTKEYS.to_string(),
        TOAST_NOTIFICATIONS.to_string(),
        DEBUG_PANEL.to_string(),
    ]
}
