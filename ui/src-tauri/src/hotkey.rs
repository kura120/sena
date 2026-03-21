use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tracing::{error, info, warn};

/// Register the overlay toggle hotkey with fallback to F12
pub fn register_overlay_hotkey(app: &tauri::App, toggle_key: &str) -> Result<(), String> {
    // Try the configured key first
    match try_register_shortcut(app, toggle_key) {
        Ok(_) => {
            info!(hotkey = toggle_key, "Overlay hotkey registered");
            Ok(())
        }
        Err(e) => {
            warn!(
                key = toggle_key,
                error = %e,
                "Configured overlay hotkey is invalid or unavailable, trying fallback F12"
            );

            // Try F12 fallback
            match try_register_shortcut(app, "F12") {
                Ok(_) => {
                    info!(hotkey = "F12", "Overlay hotkey registered (fallback)");
                    Ok(())
                }
                Err(fallback_err) => {
                    error!(
                        error = %fallback_err,
                        "Failed to register F12 fallback hotkey"
                    );
                    Err(format!(
                        "Failed to register any hotkey. Primary: {}, Fallback: {}",
                        e, fallback_err
                    ))
                }
            }
        }
    }
}

/// Try to register a specific shortcut key
fn try_register_shortcut(app: &tauri::App, key_str: &str) -> Result<(), String> {
    let shortcut: Shortcut = key_str
        .parse()
        .map_err(|e| format!("Invalid hotkey format '{}': {}", key_str, e))?;

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                if let Err(err) = crate::overlay::toggle_overlay(app_handle) {
                    error!(error = %err, "Failed to toggle overlay via hotkey");
                }
            }
        })
        .map_err(|e| format!("Failed to register hotkey '{}': {}", key_str, e))?;

    Ok(())
}
