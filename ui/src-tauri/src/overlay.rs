use crate::commands::AppState;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracing::{error, info, warn};

/// Generation counter to prevent stale hide tasks from hiding re-shown panels.
static OVERLAY_GENERATION: AtomicU64 = AtomicU64::new(0);

// Panel labels for window identification
pub const PANEL_SUBSYSTEM_HEALTH: &str = "subsystem-health";
pub const PANEL_EVENT_BUS: &str = "event-bus";
pub const PANEL_CHAT: &str = "chat";
pub const PANEL_RESOURCES: &str = "resources";
pub const PANEL_THOUGHT_STREAM: &str = "thought-stream";
pub const PANEL_MEMORY_STATS: &str = "memory-stats";
pub const PANEL_PROMPT_TRACE: &str = "prompt-trace";
pub const PANEL_CONVERSATION_TIMELINE: &str = "conversation-timeline";
pub const PANEL_TOAST: &str = "toast";
pub const PANEL_VIGNETTE: &str = "vignette";
pub const PANEL_WIDGET_BAR: &str = "widget-bar";
pub const PANEL_SETTINGS: &str = "settings";
pub const PANEL_MODEL: &str = "model-panel";

/// Panel order for staggered open animation
pub const PANEL_OPEN_ORDER: &[&str] = &[
    PANEL_SUBSYSTEM_HEALTH,
    PANEL_EVENT_BUS,
    PANEL_CHAT,
    PANEL_RESOURCES,
    PANEL_THOUGHT_STREAM,
    PANEL_MEMORY_STATS,
    PANEL_PROMPT_TRACE,
    PANEL_CONVERSATION_TIMELINE,
    PANEL_MODEL,
    PANEL_SETTINGS,
];

pub const ALL_PANELS: &[&str] = &[
    PANEL_SUBSYSTEM_HEALTH,
    PANEL_EVENT_BUS,
    PANEL_CHAT,
    PANEL_RESOURCES,
    PANEL_THOUGHT_STREAM,
    PANEL_MEMORY_STATS,
    PANEL_PROMPT_TRACE,
    PANEL_CONVERSATION_TIMELINE,
    PANEL_MODEL,
    PANEL_SETTINGS,
];

/// Payload sent to each window to coordinate animations
#[derive(Debug, Clone, Serialize)]
pub struct OverlayAnimationPayload {
    pub action: String,
    pub delay_ms: u64,
}

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

/// Try to load saved window position from the persistent store.
/// Falls back to the config position if no saved position exists.
fn load_window_position(
    app_handle: &tauri::AppHandle,
    label: &str,
    config_pos: &crate::config::WindowPosition,
) -> (f64, f64, f64, f64) {
    use tauri_plugin_store::StoreExt;

    if let Ok(store) = app_handle.store("window-positions.json") {
        if let Some(val) = store.get(label) {
            let x = val.get("x").and_then(|v| v.as_f64()).unwrap_or(config_pos.x);
            let y = val.get("y").and_then(|v| v.as_f64()).unwrap_or(config_pos.y);
            let w = val.get("width").and_then(|v| v.as_f64()).unwrap_or(config_pos.width);
            let h = val.get("height").and_then(|v| v.as_f64()).unwrap_or(config_pos.height);
            info!(label, x, y, w, h, "Loaded saved window position");
            return (x, y, w, h);
        }
    }

    (config_pos.x, config_pos.y, config_pos.width, config_pos.height)
}

/// Create all panel windows (4 debug panels + toast)
pub fn create_panel_windows(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = app.state::<AppState>();
    let overlay_config = &app_state.config.overlay;
    let handle = app.handle();

    info!("Creating overlay panel windows");

    // Subsystem Health panel
    let (hx, hy, hw, hh) = load_window_position(handle, PANEL_SUBSYSTEM_HEALTH, &overlay_config.health_window);
    let health_window = WebviewWindowBuilder::new(
        app,
        PANEL_SUBSYSTEM_HEALTH,
        WebviewUrl::App("src/windows/subsystem-health/index.html".into()),
    )
    .title("Subsystem Health")
    .inner_size(hw, hh)
    .position(hx, hy)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&health_window);

    // Event Bus panel
    let (ex, ey, ew, eh) = load_window_position(handle, PANEL_EVENT_BUS, &overlay_config.event_bus_window);
    let event_bus_window = WebviewWindowBuilder::new(
        app,
        PANEL_EVENT_BUS,
        WebviewUrl::App("src/windows/event-bus/index.html".into()),
    )
    .title("Event Bus")
    .inner_size(ew, eh)
    .position(ex, ey)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&event_bus_window);

    // Chat panel
    let (cx, cy, cw, ch) = load_window_position(handle, PANEL_CHAT, &overlay_config.chat_window);
    let chat_window = WebviewWindowBuilder::new(
        app,
        PANEL_CHAT,
        WebviewUrl::App("src/windows/chat/index.html".into()),
    )
    .title("Chat")
    .inner_size(cw, ch)
    .position(cx, cy)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&chat_window);

    // Resources panel
    let (rx, ry, rw, rh) = load_window_position(handle, PANEL_RESOURCES, &overlay_config.resources_window);
    let resources_window = WebviewWindowBuilder::new(
        app,
        PANEL_RESOURCES,
        WebviewUrl::App("src/windows/resources/index.html".into()),
    )
    .title("Resources")
    .inner_size(rw, rh)
    .position(rx, ry)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&resources_window);

    // Thought Stream panel
    let (tsx, tsy, tsw, tsh) = load_window_position(handle, PANEL_THOUGHT_STREAM, &overlay_config.thought_stream_window);
    let thought_stream_window = WebviewWindowBuilder::new(
        app,
        PANEL_THOUGHT_STREAM,
        WebviewUrl::App("src/windows/thought-stream/index.html".into()),
    )
    .title("Thought Stream")
    .inner_size(tsw, tsh)
    .position(tsx, tsy)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&thought_stream_window);

    // Memory Stats panel
    let (mx, my, mw, mh) = load_window_position(handle, PANEL_MEMORY_STATS, &overlay_config.memory_stats_window);
    let memory_stats_window = WebviewWindowBuilder::new(
        app,
        PANEL_MEMORY_STATS,
        WebviewUrl::App("src/windows/memory-stats/index.html".into()),
    )
    .title("Memory Stats")
    .inner_size(mw, mh)
    .position(mx, my)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&memory_stats_window);

    // Prompt Trace panel
    let (px, py, pw, ph) = load_window_position(handle, PANEL_PROMPT_TRACE, &overlay_config.prompt_trace_window);
    let prompt_trace_window = WebviewWindowBuilder::new(
        app,
        PANEL_PROMPT_TRACE,
        WebviewUrl::App("src/windows/prompt-trace/index.html".into()),
    )
    .title("Prompt Trace")
    .inner_size(pw, ph)
    .position(px, py)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&prompt_trace_window);

    // Conversation Timeline panel
    let (ctx, cty, ctw, cth) = load_window_position(handle, PANEL_CONVERSATION_TIMELINE, &overlay_config.conversation_timeline_window);
    let conversation_timeline_window = WebviewWindowBuilder::new(
        app,
        PANEL_CONVERSATION_TIMELINE,
        WebviewUrl::App("src/windows/conversation-timeline/index.html".into()),
    )
    .title("Conversation Timeline")
    .inner_size(ctw, cth)
    .position(ctx, cty)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&conversation_timeline_window);

    // Settings panel
    let settings_window = WebviewWindowBuilder::new(
        app,
        PANEL_SETTINGS,
        WebviewUrl::App("src/windows/settings/index.html".into()),
    )
    .title("Settings")
    .inner_size(700.0, 600.0)
    .center()
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&settings_window);

    // Model panel
    let model_panel_window = WebviewWindowBuilder::new(
        app,
        PANEL_MODEL,
        WebviewUrl::App("src/windows/model-panel/index.html".into()),
    )
    .title("Model Panel")
    .inner_size(480.0, 560.0)
    .center()
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    apply_window_effects(&model_panel_window);

    // Toast window (separate creation function but call it here)
    create_widget_bar_window(app)?;

    // Toast window (separate creation function but call it here)
    create_toast_window(app)?;

    // Vignette dim overlay (fullscreen, click-through)
    create_vignette_window(app)?;

    info!(
        panels = ALL_PANELS.len(),
        "All overlay panel windows created"
    );

    Ok(())
}

/// Compute the expected widget bar width from its layout.
/// Layout: 16px pad | 8×32px buttons + 7×2px gaps | 4+1+4 divider | 32px settings | 4+1+4 divider | 32px model | 16px pad
/// Plus flex gaps (4px) between top-level children
fn compute_widget_bar_width() -> u32 {
    let panel_buttons = 8;
    let button_size = 32;
    let panel_gap = 2;
    let panels_width = panel_buttons * button_size + (panel_buttons - 1) * panel_gap; // 270
    let divider_width = 9; // 1px + 4px margin each side
    let extra_buttons = 2; // settings + model
    let outer_padding = 32; // 16px each side
    let top_level_gap = 4;
    let top_level_items = 5; // panels, divider, settings, divider, model
    let top_level_gaps = (top_level_items - 1) * top_level_gap; // 16
    
    (panels_width + divider_width * 2 + extra_buttons * button_size + outer_padding + top_level_gaps) as u32
}

/// Calculate the widget bar position centered on the primary monitor.
/// In multi-monitor setups, the primary monitor may not start at x=0.
fn calculate_widget_bar_position(app: &tauri::AppHandle, bar_width: u32) -> (i32, i32) {
    let primary = app.primary_monitor()
        .ok()
        .flatten()
        .expect("no primary monitor detected");
    
    let monitor_width = primary.size().width;
    let scale = primary.scale_factor();
    
    let logical_monitor_width = monitor_width as f64 / scale;
    let logical_bar_width = bar_width as f64;
    
    let x = ((logical_monitor_width - logical_bar_width) / 2.0) as i32;
    let y = 16_i32;
    
    let monitor_x = primary.position().x;
    let logical_monitor_x = (monitor_x as f64 / scale) as i32;
    
    (logical_monitor_x + x, y)
}

/// Create the widget bar window — centered pill at top of screen.
/// The widget bar appears first when the overlay opens and hides last when it closes.
fn create_widget_bar_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = app.state::<AppState>();
    let overlay_config = &app_state.config.overlay;
    let handle = app.handle();

    let (_saved_x, _saved_y, widget_bar_width, _saved_height) = load_window_position(
        handle,
        PANEL_WIDGET_BAR,
        &overlay_config.widget_bar_window,
    );

    let bar_content_width = compute_widget_bar_width();
    let (widget_bar_x, widget_bar_y) = calculate_widget_bar_position(app.handle(), bar_content_width);

    // Extra height to accommodate tooltip below the bar
    let widget_bar_height = 90.0;

    let widget_bar_window = WebviewWindowBuilder::new(
        app,
        PANEL_WIDGET_BAR,
        WebviewUrl::App("src/windows/widget-bar/index.html".into()),
    )
    .title("Widget Bar")
    .inner_size(widget_bar_width, widget_bar_height)
    .position(widget_bar_x as f64, widget_bar_y as f64)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .resizable(false)
    .build()?;

    apply_window_effects(&widget_bar_window);

    info!(widget_bar_x, widget_bar_y, "Widget bar window created at centered position");
    Ok(())
}

pub const PANEL_NOTIFICATION_HISTORY: &str = "notification-history";

/// Default screen size fallback when primary monitor detection fails.
const DEFAULT_SCREEN_WIDTH: f64 = 1920.0;

/// Create the toast notification window.
/// Sized to hold a stack of up to 5 toasts (320px wide + padding, ~500px tall).
/// Positioned at top-right of the primary display.
pub fn create_toast_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let toast_width = 340.0;
    let margin = 20.0;

    let screen_width = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.size().width as f64 / m.scale_factor())
        .unwrap_or_else(|| {
            warn!(
                "Could not detect primary monitor — falling back to {DEFAULT_SCREEN_WIDTH}px width"
            );
            DEFAULT_SCREEN_WIDTH
        });
    let toast_x = screen_width - toast_width - margin;

    let toast_window = WebviewWindowBuilder::new(
        app,
        PANEL_TOAST,
        WebviewUrl::App("src/windows/toast/index.html".into()),
    )
    .title("Toast")
    .inner_size(toast_width, 1.0)
    .position(toast_x, margin)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(true) // Always visible — starts at 1px height, React resizes dynamically
    .build()?;

    // Start click-through so the 1px window never intercepts input when empty.
    // React toggles this off when toasts are visible.
    toast_window.set_ignore_cursor_events(true).map_err(|e| {
        error!(error = %e, "Failed to set toast window click-through");
        Box::new(e) as Box<dyn std::error::Error>
    })?;

    apply_window_effects(&toast_window);

    info!(screen_width, toast_x, "Toast notification window created");

    Ok(())
}

/// Create the notification history window on demand.
/// Unlike other panels, this is lazy-created when the user clicks the bell icon.
pub fn create_notification_history_window(
    app_handle: &AppHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if it already exists — just show and focus it
    if let Some(window) = app_handle.get_webview_window(PANEL_NOTIFICATION_HISTORY) {
        window.show().map_err(|e| {
            let msg = format!("Failed to show notification history: {}", e);
            error!(error = %e, "failed to show notification history");
            Box::new(std::io::Error::other(msg))
                as Box<dyn std::error::Error>
        })?;
        window.set_focus().map_err(|e| {
            let msg = format!("Failed to focus notification history: {}", e);
            error!(error = %e, "failed to focus notification history");
            Box::new(std::io::Error::other(msg))
                as Box<dyn std::error::Error>
        })?;
        return Ok(());
    }

    let history_width = 360.0;
    let history_height = 500.0;
    let margin = 20.0;

    let screen_width = app_handle
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.size().width as f64 / m.scale_factor())
        .unwrap_or(DEFAULT_SCREEN_WIDTH);
    // Place to the left of the toast column
    let history_x = screen_width - history_width - 340.0 - (margin * 2.0);

    let window = WebviewWindowBuilder::new(
        app_handle,
        PANEL_NOTIFICATION_HISTORY,
        WebviewUrl::App("src/windows/notification-history/index.html".into()),
    )
    .title("Notification History")
    .inner_size(history_width, history_height)
    .position(history_x, 100.0)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(true)
    .build()
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    apply_window_effects(&window);

    info!("Notification history window created");

    Ok(())
}

/// Show the settings window (eagerly created at startup).
#[allow(dead_code)]
pub fn create_settings_window(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(window) = app_handle.get_webview_window(PANEL_SETTINGS) {
        window.show().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        window.set_focus().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        return Ok(());
    }
    Err(Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "Settings window not found")) as Box<dyn std::error::Error>)
}

/// Show the model panel window (eagerly created at startup).
#[allow(dead_code)]
pub fn create_model_panel_window(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(window) = app_handle.get_webview_window(PANEL_MODEL) {
        window.show().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        window.set_focus().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        return Ok(());
    }
    Err(Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "Model panel window not found")) as Box<dyn std::error::Error>)
}

/// Toggle overlay visibility — if any panel is visible, hide all. Otherwise show all.
pub fn toggle_overlay(app_handle: &AppHandle) -> Result<(), String> {
    let widget_bar_visible = app_handle
        .get_webview_window(PANEL_WIDGET_BAR)
        .and_then(|win| win.is_visible().ok())
        .unwrap_or(false);

    let any_panel_visible = ALL_PANELS.iter().any(|&label| {
        app_handle
            .get_webview_window(label)
            .and_then(|win| win.is_visible().ok())
            .unwrap_or(false)
    });

    if widget_bar_visible || any_panel_visible {
        hide_all_panels(app_handle)?;
        info!("Overlay toggled: hidden");
    } else {
        show_all_panels(app_handle)?;
        info!("Overlay toggled: shown");
    }

    Ok(())
}

/// Show all panel windows with staggered animation.
///
/// 1. Show vignette first
/// 2. Show each panel in PANEL_OPEN_ORDER, each with a 60ms stagger
/// 3. Emit "overlay-animate" to each panel so it can run its CSS transition
pub fn show_all_panels(app_handle: &AppHandle) -> Result<(), String> {
    // Bump generation to cancel any pending hide task from a prior close
    OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);

    // Show vignette overlay first
    if let Some(vignette) = app_handle.get_webview_window(PANEL_VIGNETTE) {
        let _ = vignette.show();
    }

    // Emit vignette show event
    let _ = app_handle.emit(
        "overlay-animate",
        OverlayAnimationPayload {
            action: "show".to_string(),
            delay_ms: 0,
        },
    );

    // Show widget bar before panels
    if let Some(widget_bar) = app_handle.get_webview_window(PANEL_WIDGET_BAR) {
        if let Err(show_error) = widget_bar.show() {
            error!(error = %show_error, "Failed to show widget bar");
        }
    }

    // Emit widget bar show event
    let _ = app_handle.emit_to(
        PANEL_WIDGET_BAR,
        "panel-animate",
        OverlayAnimationPayload {
            action: "show".to_string(),
            delay_ms: 0,
        },
    );

    // Read panel states to determine which panels should be shown
    let panels_to_show: Vec<&str> = {
        use tauri_plugin_store::StoreExt;
        
        // Check "reopen_panels_on_toggle" setting
        let reopen_panels = app_handle
            .store("overlay-settings.json")
            .ok()
            .and_then(|store| store.get("reopen_panels_on_toggle"))
            .and_then(|val| val.as_bool())
            .unwrap_or(true); // Default: reopen panels

        if reopen_panels {
            // Show panels where panel_states[label] = true (default: true for all)
            let panel_store = app_handle.store("panel-states.json").ok();
            PANEL_OPEN_ORDER
                .iter()
                .filter(|&&label| {
                    panel_store
                        .as_ref()
                        .and_then(|store| store.get(label))
                        .and_then(|val| val.as_bool())
                        .unwrap_or(true) // Default: panel is open
                })
                .copied()
                .collect()
        } else {
            // Don't reopen any panels — just show widget bar
            Vec::new()
        }
    };

    // Show only panels that should be open, in staggered order
    for (index, &label) in panels_to_show.iter().enumerate() {
        if let Some(window) = app_handle.get_webview_window(label) {
            window.show().map_err(|e| {
                let msg = format!("Failed to show panel {}: {}", label, e);
                error!(label, error = %e, "failed to show panel");
                msg
            })?;
        } else {
            error!(label, "panel window not found");
        }

        // Emit per-window animation event with stagger delay
        let delay_ms = (index as u64) * 60;
        let _ = app_handle.emit_to(
            label,
            "panel-animate",
            OverlayAnimationPayload {
                action: "show".to_string(),
                delay_ms,
            },
        );
    }

    // Focus the chat window after showing all panels
    if let Some(chat_window) = app_handle.get_webview_window(PANEL_CHAT) {
        let _ = chat_window.set_focus();
    }

    info!("All overlay panels shown with animation");
    Ok(())
}

/// Hide all panel windows simultaneously with animation.
///
/// 1. Emit hide animation to all panels simultaneously
/// 2. Emit vignette hide
/// 3. Emit widget bar hide
/// 4. After animation duration (120ms), actually hide the windows
pub fn hide_all_panels(app_handle: &AppHandle) -> Result<(), String> {
    // Emit hide animation to all panels simultaneously (no stagger)
    for &label in ALL_PANELS {
        let _ = app_handle.emit_to(
            label,
            "panel-animate",
            OverlayAnimationPayload {
                action: "hide".to_string(),
                delay_ms: 0,
            },
        );
    }

    // Emit vignette hide
    let _ = app_handle.emit(
        "overlay-animate",
        OverlayAnimationPayload {
            action: "hide".to_string(),
            delay_ms: 0,
        },
    );

    // Emit widget bar hide
    let _ = app_handle.emit_to(
        PANEL_WIDGET_BAR,
        "panel-animate",
        OverlayAnimationPayload {
            action: "hide".to_string(),
            delay_ms: 0,
        },
    );

    // Schedule actual window hide after animation completes (120ms + buffer).
    // Capture the current generation — if show_all_panels runs before this fires,
    // the generation advances and this task becomes a no-op.
    let generation = OVERLAY_GENERATION.load(Ordering::SeqCst);
    let handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        // Bail if the overlay was re-shown since we started hiding
        if OVERLAY_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }

        for &label in ALL_PANELS {
            if let Some(window) = handle.get_webview_window(label) {
                let _ = window.hide();
            }
        }

        // Hide vignette after panels are hidden
        if let Some(vignette) = handle.get_webview_window(PANEL_VIGNETTE) {
            let _ = vignette.hide();
        }

        // Best-effort: widget bar may already be hidden
        if let Some(widget_bar) = handle.get_webview_window(PANEL_WIDGET_BAR) {
            let _ = widget_bar.hide();
        }
    });

    info!("All overlay panels hiding with animation");
    Ok(())
}

/// Show a single panel window and persist its open state.
pub fn show_single_panel(app_handle: &AppHandle, label: &str) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    if let Some(window) = app_handle.get_webview_window(label) {
        window.show().map_err(|e| {
            let msg = format!("Failed to show panel {}: {}", label, e);
            error!(label, error = %e, "failed to show panel");
            msg
        })?;
        window.set_focus().map_err(|e| {
            let msg = format!("Failed to focus panel {}: {}", label, e);
            error!(label, error = %e, "failed to focus panel");
            msg
        })?;
    } else {
        return Err(format!("Panel window '{}' not found", label));
    }

    // Persist state
    if let Ok(store) = app_handle.store("panel-states.json") {
        store.set(label.to_string(), serde_json::json!(true));
        if let Err(save_error) = store.save() {
            error!(label, error = %save_error, "Failed to persist panel open state");
        }
    }

    // Notify widget bar (non-critical broadcast)
    let _ = app_handle.emit("panel-state-changed", serde_json::json!({
        "label": label,
        "is_open": true,
    }));

    info!(label, "Single panel shown");
    Ok(())
}

/// Hide a single panel window with animation and persist its closed state.
///
/// The flow mirrors `hide_all_panels`:
/// 1. Emit `panel-animate` hide event so the frontend plays the CSS transition
/// 2. Immediately persist state and notify widget bar
/// 3. After 150ms, actually hide the window
pub fn hide_single_panel(app_handle: &AppHandle, label: &str) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;

    if app_handle.get_webview_window(label).is_none() {
        return Err(format!("Panel window '{}' not found", label));
    }

    // Emit hide animation event to the panel
    let _ = app_handle.emit_to(
        label,
        "panel-animate",
        OverlayAnimationPayload {
            action: "hide".to_string(),
            delay_ms: 0,
        },
    );

    // Persist state immediately so widget bar updates without waiting for animation
    if let Ok(store) = app_handle.store("panel-states.json") {
        store.set(label.to_string(), serde_json::json!(false));
        if let Err(save_error) = store.save() {
            error!(label, error = %save_error, "Failed to persist panel closed state");
        }
    }

    // Notify widget bar (non-critical broadcast)
    let _ = app_handle.emit("panel-state-changed", serde_json::json!({
        "label": label,
        "is_open": false,
    }));

    // Schedule actual window hide after animation completes (120ms transition + buffer)
    let handle = app_handle.clone();
    let label_owned = label.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

        if let Some(window) = handle.get_webview_window(&label_owned) {
            if let Err(e) = window.hide() {
                error!(label = %label_owned, error = %e, "Failed to hide panel after animation");
            }
        }
    });

    info!(label, "Single panel hiding with animation");
    Ok(())
}

/// Create the vignette dim overlay window (fullscreen, click-through).
fn create_vignette_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let vignette_window = WebviewWindowBuilder::new(
        app,
        PANEL_VIGNETTE,
        WebviewUrl::App("src/windows/vignette/index.html".into()),
    )
    .title("Vignette")
    .maximized(true)
    .always_on_top(true)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    // Make the vignette click-through so all mouse events pass to windows behind it
    vignette_window
        .set_ignore_cursor_events(true)
        .map_err(|e| {
            error!(error = %e, "Failed to set vignette as click-through");
            Box::new(e) as Box<dyn std::error::Error>
        })?;

    info!("Vignette overlay window created (maximized, click-through)");
    Ok(())
}
