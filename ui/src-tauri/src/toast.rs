use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter};
use tracing::{error, info};

static TOAST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Payload emitted to the "toasts" window via the "toast" event
#[derive(Debug, Clone, Serialize)]
pub struct ToastPayload {
    pub id: String,
    pub toast_type: String,
    pub title: String,
    pub message: String,
    pub dismiss_ms: u64,
    pub dismissible: bool,
    pub timestamp: u64,
}

/// Emit a toast notification to the "toasts" window.
///
/// This is the primary API for all backend code to show user-facing
/// notifications. It emits a "toast" event that the React toast window
/// subscribes to.
pub fn emit_toast(app: &AppHandle, toast_type: &str, title: &str, message: &str) {
    emit_toast_with_options(app, toast_type, title, message, 5000, true);
}

/// Emit a toast with custom dismiss timeout and dismissible flag.
pub fn emit_toast_with_options(
    app: &AppHandle,
    toast_type: &str,
    title: &str,
    message: &str,
    dismiss_ms: u64,
    dismissible: bool,
) {
    let id = generate_toast_id();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let payload = ToastPayload {
        id,
        toast_type: toast_type.to_string(),
        title: title.to_string(),
        message: message.to_string(),
        dismiss_ms,
        dismissible,
        timestamp,
    };

    info!(
        toast_type = %toast_type,
        title = %title,
        "Emitting toast notification"
    );

    if let Err(e) = app.emit("toast", &payload) {
        error!(error = %e, "Failed to emit toast event");
    }

    // Persist to notification-history store so the history panel can read on mount
    persist_to_history_store(app, &payload);
}

/// Persist a toast to the notification-history store so it's available
/// when the notification history panel opens later.
fn persist_to_history_store(app: &AppHandle, toast: &ToastPayload) {
    use tauri_plugin_store::StoreExt;

    let store = match app.store("notification-history.json") {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "notification-history store not available");
            return;
        }
    };

    let key = "notification-history";
    let mut history: Vec<serde_json::Value> = store
        .get(key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Prepend new toast (most recent first)
    if let Ok(toast_json) = serde_json::to_value(toast) {
        history.insert(0, toast_json);
    }

    // Cap at 50 entries
    history.truncate(50);

    store.set(key.to_string(), serde_json::json!(history));

    if let Err(e) = store.save() {
        tracing::debug!(error = %e, "failed to persist notification history");
    }
}

/// Generate a unique toast ID using timestamp + atomic counter
fn generate_toast_id() -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let counter = TOAST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("toast-{}-{}", timestamp, counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toast_id_uniqueness() {
        let id1 = generate_toast_id();
        let id2 = generate_toast_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("toast-"));
    }

    #[test]
    fn test_toast_payload_serialization() {
        let payload = ToastPayload {
            id: "toast-123".to_string(),
            toast_type: "info".to_string(),
            title: "Test".to_string(),
            message: "Hello".to_string(),
            dismiss_ms: 5000,
            dismissible: true,
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&payload).expect("Failed to serialize");
        assert!(json.contains("\"toast_type\":\"info\""));
        assert!(json.contains("\"dismiss_ms\":5000"));
    }
}
