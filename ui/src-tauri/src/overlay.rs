use crate::commands::AppState;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracing::{error, info, warn};

// Panel labels for window identification
pub const PANEL_SUBSYSTEM_HEALTH: &str = "subsystem-health";
pub const PANEL_EVENT_BUS: &str = "event-bus";
pub const PANEL_CHAT: &str = "chat";
pub const PANEL_BOOT_TIMELINE: &str = "boot-timeline";
pub const PANEL_TOAST: &str = "toast";

pub const ALL_PANELS: &[&str] = &[
    PANEL_SUBSYSTEM_HEALTH,
    PANEL_EVENT_BUS,
    PANEL_CHAT,
    PANEL_BOOT_TIMELINE,
];

/// Apply platform-specific visual effects to a window.
/// On Windows, applies acrylic blur for translucent background.
/// On macOS/Linux, this is a no-op (handled differently or not supported).
#[cfg(target_os = "windows")]
fn apply_window_effects(window: &WebviewWindow) {
    // Tauri v2 window effects API
    // Note: The exact API may vary based on Tauri version.
    // As of Tauri v2 RC, window effects are applied via set_effects() method.
    // However, this API is still evolving. For now, we document the intent.
    
    // TODO: Add window effects when Tauri v2 stabilizes the API
    // Expected usage (when ready):
    // use tauri::window::{Effect, EffectsBuilder};
    // let effects = EffectsBuilder::new()
    //     .effect(Effect::Acrylic)
    //     .build();
    // if let Err(e) = window.set_effects(effects) {
    //     warn!(window_label = ?window.label(), error = %e, "Failed to apply window effects");
    // }
    
    warn!(
        window_label = ?window.label(),
        "Window effects not yet implemented — waiting for Tauri v2 API stabilization"
    );
}

#[cfg(not(target_os = "windows"))]
fn apply_window_effects(_window: &WebviewWindow) {
    // No-op on non-Windows platforms
    // macOS uses different vibrancy effects
    // Linux has no built-in blur support
}

/// Create all panel windows (4 debug panels + toast)
pub fn create_panel_windows(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = app.state::<AppState>();
    let overlay_config = &app_state.config.overlay;

    info!("Creating overlay panel windows");

    // Subsystem Health panel
    let health_window = WebviewWindowBuilder::new(
        app,
        PANEL_SUBSYSTEM_HEALTH,
        WebviewUrl::App("src/windows/subsystem-health/index.html".into()),
    )
    .title("Subsystem Health")
    .inner_size(
        overlay_config.health_window.width,
        overlay_config.health_window.height,
    )
    .position(
        overlay_config.health_window.x,
        overlay_config.health_window.y,
    )
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;
    
    apply_window_effects(&health_window);

    // Event Bus panel
    let event_bus_window = WebviewWindowBuilder::new(
        app,
        PANEL_EVENT_BUS,
        WebviewUrl::App("src/windows/event-bus/index.html".into()),
    )
    .title("Event Bus")
    .inner_size(
        overlay_config.event_bus_window.width,
        overlay_config.event_bus_window.height,
    )
    .position(
        overlay_config.event_bus_window.x,
        overlay_config.event_bus_window.y,
    )
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;
    
    apply_window_effects(&event_bus_window);

    // Chat panel
    let chat_window = WebviewWindowBuilder::new(
        app,
        PANEL_CHAT,
        WebviewUrl::App("src/windows/chat/index.html".into()),
    )
    .title("Chat")
    .inner_size(
        overlay_config.chat_window.width,
        overlay_config.chat_window.height,
    )
    .position(overlay_config.chat_window.x, overlay_config.chat_window.y)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;
    
    apply_window_effects(&chat_window);

    // Boot Timeline panel
    let boot_timeline_window = WebviewWindowBuilder::new(
        app,
        PANEL_BOOT_TIMELINE,
        WebviewUrl::App("src/windows/boot-timeline/index.html".into()),
    )
    .title("Boot Timeline")
    .inner_size(
        overlay_config.boot_timeline_window.width,
        overlay_config.boot_timeline_window.height,
    )
    .position(
        overlay_config.boot_timeline_window.x,
        overlay_config.boot_timeline_window.y,
    )
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;
    
    apply_window_effects(&boot_timeline_window);

    // Toast window (separate creation function but call it here)
    create_toast_window(app)?;

    info!(
        panels = ALL_PANELS.len(),
        "All overlay panel windows created"
    );

    Ok(())
}

/// Create the toast notification window
pub fn create_toast_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Toast window positioned at top-right with fixed size
    let toast_window = WebviewWindowBuilder::new(
        app,
        PANEL_TOAST,
        WebviewUrl::App("src/windows/toast/index.html".into()),
    )
    .title("Toast")
    .inner_size(280.0, 56.0)
    .position(1620.0, 20.0) // Top-right for typical 1920x1080 display
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;
    
    apply_window_effects(&toast_window);

    info!("Toast notification window created");

    Ok(())
}

/// Toggle overlay visibility — if any panel is visible, hide all. Otherwise show all.
pub fn toggle_overlay(app_handle: &AppHandle) -> Result<(), String> {
    let any_visible = ALL_PANELS.iter().any(|&label| {
        app_handle
            .get_webview_window(label)
            .and_then(|win| win.is_visible().ok())
            .unwrap_or(false)
    });

    if any_visible {
        hide_all_panels(app_handle)?;
        info!("Overlay toggled: hidden");
    } else {
        show_all_panels(app_handle)?;
        info!("Overlay toggled: shown");
    }

    Ok(())
}

/// Show all panel windows and bring to front
pub fn show_all_panels(app_handle: &AppHandle) -> Result<(), String> {
    for &label in ALL_PANELS {
        if let Some(window) = app_handle.get_webview_window(label) {
            window.show().map_err(|e| {
                let msg = format!("Failed to show panel {}: {}", label, e);
                error!(label, error = %e, "failed to show panel");
                msg
            })?;
        } else {
            error!(label, "panel window not found");
        }
    }

    // Focus the chat window after showing all panels
    if let Some(chat_window) = app_handle.get_webview_window(PANEL_CHAT) {
        chat_window.set_focus().map_err(|e| {
            let msg = format!("Failed to focus chat window: {}", e);
            error!(error = %e, "failed to focus chat window");
            msg
        })?;
    }

    info!("All overlay panels shown");
    Ok(())
}

/// Hide all panel windows
pub fn hide_all_panels(app_handle: &AppHandle) -> Result<(), String> {
    for &label in ALL_PANELS {
        if let Some(window) = app_handle.get_webview_window(label) {
            window.hide().map_err(|e| {
                let msg = format!("Failed to hide panel {}: {}", label, e);
                error!(label, error = %e, "failed to hide panel");
                msg
            })?;
        }
    }

    info!("All overlay panels hidden");
    Ok(())
}
