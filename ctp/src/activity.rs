//! Activity state detection — abstracts idle detection behind a trait.
//!
//! On Windows, `GetLastInputInfo` is used to detect user idle time.
//! On other platforms, a stub that always reports `UserActive` is provided.
//! The hot path (`current_state()`) reads only an atomic — zero Win32 calls.

use crate::config::{ActivityConfig, SurfaceThresholds};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// User activity state — determines surface threshold and consolidation eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ActivityState {
    UserActive = 0,
    Idle2Min = 1,
    Idle10Min = 2,
}

impl ActivityState {
    /// Convert enum to u8 for atomic storage.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Return the surface threshold for this activity state from config.
    pub fn surface_threshold(self, thresholds: &SurfaceThresholds) -> f32 {
        match self {
            ActivityState::UserActive => thresholds.user_active,
            ActivityState::Idle2Min => thresholds.idle_2min,
            ActivityState::Idle10Min => thresholds.idle_10min,
        }
    }

    /// Returns true only for deep idle — `Idle10Min`.
    pub fn is_deep_idle(self) -> bool {
        matches!(self, ActivityState::Idle10Min)
    }
}

impl From<u8> for ActivityState {
    fn from(value: u8) -> Self {
        match value {
            0 => ActivityState::UserActive,
            1 => ActivityState::Idle2Min,
            2 => ActivityState::Idle10Min,
            _ => ActivityState::UserActive,
        }
    }
}

/// Trait for platform-specific idle duration detection.
///
/// Implementations return the number of milliseconds since the last user input.
/// Must be Send + Sync for use across async tasks.
pub trait ActivityDetector: Send + Sync + 'static {
    fn idle_duration_ms(&self) -> u64;
}

/// Win32 idle detector — uses `GetLastInputInfo` + `GetTickCount`.
#[cfg(target_os = "windows")]
pub struct WindowsActivityDetector;

#[cfg(target_os = "windows")]
impl WindowsActivityDetector {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "windows")]
impl ActivityDetector for WindowsActivityDetector {
    fn idle_duration_ms(&self) -> u64 {
        use windows::Win32::System::SystemInformation::GetTickCount;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };

        // Safety: `GetLastInputInfo` is a well-documented Win32 API that writes
        // into the provided `LASTINPUTINFO` struct. The struct is stack-allocated
        // with correct `cbSize` and the pointer is valid for the duration of the call.
        let success = unsafe { GetLastInputInfo(&mut info) };

        if !success.as_bool() {
            return 0;
        }

        // Safety: `GetTickCount` is a Win32 API with no parameters and no
        // memory safety implications.
        let tick_now = unsafe { GetTickCount() };

        // Handle tick counter wraparound (every ~49.7 days)
        tick_now.wrapping_sub(info.dwTime) as u64
    }
}

/// Stub detector for non-Windows platforms — always reports UserActive (0 ms idle).
#[cfg(not(target_os = "windows"))]
pub struct StubActivityDetector;

#[cfg(not(target_os = "windows"))]
impl StubActivityDetector {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(target_os = "windows"))]
impl ActivityDetector for StubActivityDetector {
    fn idle_duration_ms(&self) -> u64 {
        0
    }
}

/// Creates the platform-appropriate activity detector.
pub fn create_platform_detector() -> Arc<dyn ActivityDetector> {
    #[cfg(target_os = "windows")]
    {
        Arc::new(WindowsActivityDetector::new())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Arc::new(StubActivityDetector::new())
    }
}

/// Background activity monitor. Polls the platform detector on a dedicated
/// task and caches the result as an `AtomicU8`. The hot path reads only the
/// atomic — never calls Win32 directly.
pub struct ActivityMonitor {
    cached_state: Arc<AtomicU8>,
}

impl ActivityMonitor {
    pub fn new() -> Self {
        Self {
            cached_state: Arc::new(AtomicU8::new(ActivityState::UserActive.as_u8())),
        }
    }

    /// Read the current activity state from the atomic cache.
    /// This is the hot path — zero platform calls.
    pub fn current_state(&self) -> ActivityState {
        ActivityState::from(self.cached_state.load(Ordering::Relaxed))
    }

    /// Directly set the cached state — used for testing.
    #[cfg(test)]
    pub fn set_state(&self, state: ActivityState) {
        self.cached_state.store(state.as_u8(), Ordering::Relaxed);
    }

    /// Get a reference to the inner atomic for sharing with the poll loop.
    pub fn cached_state_ref(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.cached_state)
    }

    /// Run the background poll loop. This is the ONE place where
    /// `tokio::time::sleep` is acceptable — it is the activity monitor,
    /// not the thought loop.
    pub async fn run_poll_loop(&self, config: &ActivityConfig) {
        let detector = create_platform_detector();
        let poll_interval =
            std::time::Duration::from_millis(config.poll_interval_ms);
        let idle_2min_threshold_ms = config.idle_2min_threshold_secs * 1000;
        let idle_10min_threshold_ms = config.idle_10min_threshold_secs * 1000;

        loop {
            tokio::time::sleep(poll_interval).await;

            let detector_clone = Arc::clone(&detector);
            let idle_ms = tokio::task::spawn_blocking(move || {
                detector_clone.idle_duration_ms()
            })
            .await
            .unwrap_or(0);

            let new_state = if idle_ms >= idle_10min_threshold_ms {
                ActivityState::Idle10Min
            } else if idle_ms >= idle_2min_threshold_ms {
                ActivityState::Idle2Min
            } else {
                ActivityState::UserActive
            };

            self.cached_state
                .store(new_state.as_u8(), Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_state_from_u8_roundtrip() {
        let states = [
            ActivityState::UserActive,
            ActivityState::Idle2Min,
            ActivityState::Idle10Min,
        ];
        for state in states {
            assert_eq!(
                ActivityState::from(state.as_u8()),
                state,
                "roundtrip failed for {:?}",
                state
            );
        }
    }

    #[test]
    fn test_surface_threshold_per_state() {
        let thresholds = SurfaceThresholds {
            user_active: 0.9,
            idle_2min: 0.6,
            idle_10min: 0.3,
        };

        assert_eq!(
            ActivityState::UserActive.surface_threshold(&thresholds),
            0.9
        );
        assert_eq!(ActivityState::Idle2Min.surface_threshold(&thresholds), 0.6);
        assert_eq!(
            ActivityState::Idle10Min.surface_threshold(&thresholds),
            0.3
        );
    }

    #[test]
    fn test_is_deep_idle_only_for_idle_10min() {
        assert!(!ActivityState::UserActive.is_deep_idle());
        assert!(!ActivityState::Idle2Min.is_deep_idle());
        assert!(ActivityState::Idle10Min.is_deep_idle());
    }

    #[test]
    fn test_atomic_cache_updates() {
        let monitor = ActivityMonitor::new();

        // Default is UserActive
        assert_eq!(monitor.current_state(), ActivityState::UserActive);

        // Update to Idle2Min
        monitor.set_state(ActivityState::Idle2Min);
        assert_eq!(monitor.current_state(), ActivityState::Idle2Min);

        // Update to Idle10Min
        monitor.set_state(ActivityState::Idle10Min);
        assert_eq!(monitor.current_state(), ActivityState::Idle10Min);

        // Back to UserActive
        monitor.set_state(ActivityState::UserActive);
        assert_eq!(monitor.current_state(), ActivityState::UserActive);
    }

    #[test]
    fn test_unknown_u8_defaults_to_user_active() {
        // Values beyond the enum range should default to UserActive
        assert_eq!(ActivityState::from(255), ActivityState::UserActive);
        assert_eq!(ActivityState::from(3), ActivityState::UserActive);
    }
}
