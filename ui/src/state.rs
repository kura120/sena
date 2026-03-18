use std::sync::Arc;
use tokio::sync::RwLock;

use crate::debug_state::DebugState;

/// Global application state provided as context to all components.
#[derive(Clone)]
pub struct AppState {
    pub debug_panel_open: bool,
    pub debug_state: Arc<RwLock<DebugState>>,
}

impl AppState {
    /// Create a new AppState with default values.
    pub fn new(debug_state: DebugState) -> Self {
        Self {
            debug_panel_open: false,
            debug_state: Arc::new(RwLock::new(debug_state)),
        }
    }

    /// Toggle the debug panel open/closed.
    pub fn toggle_debug_panel(&mut self) {
        self.debug_panel_open = !self.debug_panel_open;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(DebugState::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_default_panel_hidden() {
        let state = AppState::default();
        assert!(
            !state.debug_panel_open,
            "debug panel should be hidden by default"
        );
    }

    #[test]
    fn test_toggle_panel_flips_state() {
        let mut state = AppState::default();
        assert!(!state.debug_panel_open);
        state.toggle_debug_panel();
        assert!(state.debug_panel_open);
        state.toggle_debug_panel();
        assert!(!state.debug_panel_open);
    }
}
