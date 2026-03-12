//! Hardware capability detection for model-probe.
//!
//! Detects VRAM, system RAM, CUDA compute capability, and derives the
//! `HardwareTier` used by all downstream subsystems to select degradation
//! levels. All tier thresholds come from config — never hardcoded here.

use crate::config::HardwareConfig;
use crate::error::{ErrorCode, SenaError, SenaResult};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Hardware tier classification. Published alongside ModelCapabilityProfile
/// so every subsystem can select its resource strategy at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HardwareTier {
    Low,
    Mid,
    High,
}

impl std::fmt::Display for HardwareTier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HardwareTier::Low => write!(formatter, "Low"),
            HardwareTier::Mid => write!(formatter, "Mid"),
            HardwareTier::High => write!(formatter, "High"),
        }
    }
}

/// Snapshot of the machine's hardware relevant to model inference.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HardwareProfile {
    /// Total GPU VRAM in megabytes.
    pub vram_total_mb: u64,
    /// Available (free) GPU VRAM in megabytes at detection time.
    pub vram_available_mb: u64,
    /// Total system RAM in megabytes.
    pub ram_total_mb: u64,
    /// CUDA compute capability string, e.g. "8.6" for Ampere. Empty if unavailable.
    pub cuda_compute: String,
    /// Derived hardware tier based on VRAM thresholds from config.
    pub tier: HardwareTier,
}

// ─────────────────────────────────────────────────────────────────────────────
// Detection
// ─────────────────────────────────────────────────────────────────────────────

/// Detect hardware capabilities and derive the tier.
///
/// Uses `spawn_blocking` internally because NVML and sysinfo perform
/// synchronous OS/driver calls that must not block the async runtime.
pub async fn detect_hardware(hardware_config: &HardwareConfig) -> SenaResult<HardwareProfile> {
    let hardware_config_clone = hardware_config.clone();

    tokio::task::spawn_blocking(move || detect_hardware_blocking(&hardware_config_clone))
        .await
        .map_err(|join_error| {
            SenaError::new(
                ErrorCode::HardwareDetectionFailed,
                "hardware detection task was cancelled or panicked",
            )
            .with_debug_context(format!("JoinError: {join_error}"))
        })?
}

/// Synchronous hardware detection — runs inside `spawn_blocking`.
fn detect_hardware_blocking(hardware_config: &HardwareConfig) -> SenaResult<HardwareProfile> {
    let gpu_info = detect_gpu_info();
    let ram_total_mb = detect_system_ram_mb();

    let (vram_total_mb, vram_available_mb, cuda_compute) = match gpu_info {
        Ok(info) => {
            tracing::info!(
                subsystem = "model_probe",
                event_type = "gpu_detected",
                vram_total_mb = info.vram_total_mb,
                vram_available_mb = info.vram_available_mb,
                cuda_compute = %info.cuda_compute,
                "NVIDIA GPU detected"
            );
            (info.vram_total_mb, info.vram_available_mb, info.cuda_compute)
        }
        Err(gpu_error) => {
            // GPU detection failure is not fatal — system may be CPU-only.
            // Log the error and fall through to Low tier.
            tracing::warn!(
                subsystem = "model_probe",
                event_type = "gpu_detection_failed",
                error_code = %gpu_error.code,
                error_message = %gpu_error.message,
                "GPU detection failed — assuming no dedicated GPU"
            );
            (0, 0, String::new())
        }
    };

    let tier = derive_hardware_tier(vram_total_mb, hardware_config);

    let profile = HardwareProfile {
        vram_total_mb,
        vram_available_mb,
        ram_total_mb,
        cuda_compute,
        tier,
    };

    tracing::info!(
        subsystem = "model_probe",
        event_type = "hardware_profile_built",
        vram_total_mb = profile.vram_total_mb,
        vram_available_mb = profile.vram_available_mb,
        ram_total_mb = profile.ram_total_mb,
        cuda_compute = %profile.cuda_compute,
        tier = %profile.tier,
        "hardware profile assembled"
    );

    Ok(profile)
}

// ─────────────────────────────────────────────────────────────────────────────
// GPU detection via NVML
// ─────────────────────────────────────────────────────────────────────────────

struct GpuInfo {
    vram_total_mb: u64,
    vram_available_mb: u64,
    cuda_compute: String,
}

/// Detect NVIDIA GPU properties via NVML.
///
/// Uses the first GPU device (index 0). Multi-GPU setups are out of scope
/// for V1 — Sena targets single-GPU consumer hardware.
fn detect_gpu_info() -> SenaResult<GpuInfo> {
    let nvml = nvml_wrapper::Nvml::init().map_err(|nvml_error| {
        SenaError::new(
            ErrorCode::HardwareDetectionFailed,
            "failed to initialize NVML — NVIDIA driver may not be installed",
        )
        .with_debug_context(format!("NVML init error: {nvml_error}"))
    })?;

    let device = nvml.device_by_index(0).map_err(|nvml_error| {
        SenaError::new(
            ErrorCode::HardwareDetectionFailed,
            "failed to get GPU device at index 0",
        )
        .with_debug_context(format!("NVML device error: {nvml_error}"))
    })?;

    let memory_info = device.memory_info().map_err(|nvml_error| {
        SenaError::new(
            ErrorCode::HardwareDetectionFailed,
            "failed to query GPU memory info",
        )
        .with_debug_context(format!("NVML memory info error: {nvml_error}"))
    })?;

    let bytes_to_mb = |bytes: u64| bytes / (1024 * 1024);

    let vram_total_mb = bytes_to_mb(memory_info.total);
    let vram_available_mb = bytes_to_mb(memory_info.free);

    let cuda_compute = match device.cuda_compute_capability() {
        Ok(capability) => format!("{}.{}", capability.major, capability.minor),
        Err(nvml_error) => {
            tracing::warn!(
                subsystem = "model_probe",
                event_type = "cuda_compute_unavailable",
                error = %nvml_error,
                "could not read CUDA compute capability — field will be empty"
            );
            String::new()
        }
    };

    Ok(GpuInfo {
        vram_total_mb,
        vram_available_mb,
        cuda_compute,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// System RAM detection
// ─────────────────────────────────────────────────────────────────────────────

/// Detect total system RAM in megabytes via `sysinfo`.
fn detect_system_ram_mb() -> u64 {
    use sysinfo::System;

    let mut system = System::new();
    system.refresh_memory();

    let total_bytes = system.total_memory();
    // sysinfo returns bytes on all platforms
    total_bytes / (1024 * 1024)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tier derivation
// ─────────────────────────────────────────────────────────────────────────────

/// Derive `HardwareTier` from total VRAM using config-driven thresholds.
///
/// PRD §2.2 defaults: Low < 8192 MB, Mid 8192–16383 MB, High >= 16384 MB.
/// The actual boundary values come from `[hardware]` in the config TOML so
/// they can be changed without recompilation.
pub fn derive_hardware_tier(vram_total_mb: u64, hardware_config: &HardwareConfig) -> HardwareTier {
    if vram_total_mb >= hardware_config.high_tier_vram_floor_mb {
        HardwareTier::High
    } else if vram_total_mb >= hardware_config.low_tier_vram_ceiling_mb {
        HardwareTier::Mid
    } else {
        HardwareTier::Low
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_hardware_config() -> HardwareConfig {
        HardwareConfig {
            low_tier_vram_ceiling_mb: 8192,
            high_tier_vram_floor_mb: 16384,
        }
    }

    #[test]
    fn tier_low_when_no_vram() {
        let tier = derive_hardware_tier(0, &default_hardware_config());
        assert_eq!(tier, HardwareTier::Low);
    }

    #[test]
    fn tier_low_when_below_mid_threshold() {
        let tier = derive_hardware_tier(4096, &default_hardware_config());
        assert_eq!(tier, HardwareTier::Low);
    }

    #[test]
    fn tier_low_at_boundary_just_below_mid() {
        let tier = derive_hardware_tier(8191, &default_hardware_config());
        assert_eq!(tier, HardwareTier::Low);
    }

    #[test]
    fn tier_mid_at_exact_mid_threshold() {
        let tier = derive_hardware_tier(8192, &default_hardware_config());
        assert_eq!(tier, HardwareTier::Mid);
    }

    #[test]
    fn tier_mid_between_thresholds() {
        let tier = derive_hardware_tier(12288, &default_hardware_config());
        assert_eq!(tier, HardwareTier::Mid);
    }

    #[test]
    fn tier_mid_at_boundary_just_below_high() {
        let tier = derive_hardware_tier(16383, &default_hardware_config());
        assert_eq!(tier, HardwareTier::Mid);
    }

    #[test]
    fn tier_high_at_exact_high_threshold() {
        let tier = derive_hardware_tier(16384, &default_hardware_config());
        assert_eq!(tier, HardwareTier::High);
    }

    #[test]
    fn tier_high_well_above_threshold() {
        let tier = derive_hardware_tier(24576, &default_hardware_config());
        assert_eq!(tier, HardwareTier::High);
    }

    #[test]
    fn tier_respects_custom_thresholds() {
        let custom = HardwareConfig {
            low_tier_vram_ceiling_mb: 4000,
            high_tier_vram_floor_mb: 8000,
        };
        assert_eq!(derive_hardware_tier(3999, &custom), HardwareTier::Low);
        assert_eq!(derive_hardware_tier(4000, &custom), HardwareTier::Mid);
        assert_eq!(derive_hardware_tier(7999, &custom), HardwareTier::Mid);
        assert_eq!(derive_hardware_tier(8000, &custom), HardwareTier::High);
    }

    #[test]
    fn hardware_profile_serializes_to_json() {
        let profile = HardwareProfile {
            vram_total_mb: 8192,
            vram_available_mb: 6000,
            ram_total_mb: 32768,
            cuda_compute: "8.6".to_string(),
            tier: HardwareTier::Mid,
        };

        let json = serde_json::to_string(&profile);
        assert!(json.is_ok(), "HardwareProfile must serialize to JSON");

        let deserialized: Result<HardwareProfile, _> =
            serde_json::from_str(&json.expect("serialization confirmed ok above"));
        assert!(
            deserialized.is_ok(),
            "HardwareProfile must round-trip through JSON"
        );
    }

    #[test]
    fn hardware_tier_display() {
        assert_eq!(format!("{}", HardwareTier::Low), "Low");
        assert_eq!(format!("{}", HardwareTier::Mid), "Mid");
        assert_eq!(format!("{}", HardwareTier::High), "High");
    }
}
