//! Capability constants for the memory-engine subsystem.
//!
//! These capability strings are reported to daemon-bus when the subsystem
//! signals ready. They describe what features this subsystem provides.

/// Write operations to memory tiers
pub const MEMORY_WRITE: &str = "memory_write";

/// Read operations from memory tiers
pub const MEMORY_READ: &str = "memory_read";

/// Automatic tier promotion based on access patterns
pub const TIER_PROMOTION: &str = "tier_promotion";

/// Model profile-aware configuration management
pub const PROFILE_MANAGEMENT: &str = "profile_management";

/// Subscribe to consolidation requests from CTP
pub const CONSOLIDATION_SUBSCRIPTION: &str = "consolidation_subscription";

/// Returns the list of capabilities the memory-engine subsystem currently provides.
///
/// This is called when signaling MEMORY_ENGINE_READY to daemon-bus.
pub fn get_capabilities() -> Vec<String> {
    vec![
        MEMORY_WRITE.to_string(),
        MEMORY_READ.to_string(),
        TIER_PROMOTION.to_string(),
        PROFILE_MANAGEMENT.to_string(),
        CONSOLIDATION_SUBSCRIPTION.to_string(),
    ]
}
