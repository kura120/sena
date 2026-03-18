/// Colour constants for the debug panel UI.
/// All colours defined here — never inline in components.
/// Uses `(u8, u8, u8)` tuples for Freya's `Color` type.

/// Status indicator colours.
pub const STATUS_READY_COLOR: (u8, u8, u8) = (76, 175, 80);
pub const STATUS_DEGRADED_COLOR: (u8, u8, u8) = (255, 193, 7);
pub const STATUS_UNAVAILABLE_COLOR: (u8, u8, u8) = (244, 67, 54);
pub const STATUS_UNKNOWN_COLOR: (u8, u8, u8) = (158, 158, 158);

/// VRAM bar colours.
pub const VRAM_NORMAL_COLOR: (u8, u8, u8) = (76, 175, 80);
pub const VRAM_WARNING_COLOR: (u8, u8, u8) = (255, 193, 7);
pub const VRAM_CRITICAL_COLOR: (u8, u8, u8) = (244, 67, 54);

/// VRAM bar thresholds (as fractions 0.0–1.0).
pub const VRAM_WARNING_THRESHOLD: f32 = 0.80;
pub const VRAM_CRITICAL_THRESHOLD: f32 = 0.95;

/// Thought score badge colours.
pub const SCORE_HIGH_COLOR: (u8, u8, u8) = (76, 175, 80);
pub const SCORE_MEDIUM_COLOR: (u8, u8, u8) = (255, 193, 7);
pub const SCORE_LOW_COLOR: (u8, u8, u8) = (158, 158, 158);

/// Thought score thresholds.
pub const SCORE_HIGH_THRESHOLD: f32 = 0.8;
pub const SCORE_MEDIUM_THRESHOLD: f32 = 0.5;

/// Panel background and text colours.
pub const PANEL_BACKGROUND: (u8, u8, u8) = (30, 30, 30);
pub const PANEL_BORDER: (u8, u8, u8) = (60, 60, 60);
pub const PANEL_HEADER_BACKGROUND: (u8, u8, u8) = (40, 40, 40);
pub const TEXT_PRIMARY: (u8, u8, u8) = (224, 224, 224);
pub const TEXT_SECONDARY: (u8, u8, u8) = (158, 158, 158);
pub const TEXT_MUTED: (u8, u8, u8) = (100, 100, 100);

/// Main app background.
pub const APP_BACKGROUND: (u8, u8, u8) = (18, 18, 18);
pub const HEADER_BACKGROUND: (u8, u8, u8) = (28, 28, 28);

/// VRAM bar background (track).
pub const VRAM_BAR_TRACK: (u8, u8, u8) = (50, 50, 50);

/// Section header.
pub const SECTION_HEADER_COLOR: (u8, u8, u8) = (180, 180, 180);
