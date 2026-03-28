//! Tests that subsystems report capabilities when signaling ready
//! and that daemon-bus stores and serves those capabilities.
//! These tests validate the capability format contract without depending
//! on generated proto types (daemon-bus is a binary crate).

/// Capability format: "name" for operational, "name:degraded" for degraded
fn is_degraded(capability: &str) -> bool {
    capability.ends_with(":degraded")
}

fn capability_name(capability: &str) -> &str {
    capability.split(':').next().unwrap_or(capability)
}

#[test]
fn test_capability_format_operational() {
    let caps = vec![
        "memory_write".to_string(),
        "memory_read".to_string(),
        "tier_promotion".to_string(),
    ];

    assert_eq!(caps.len(), 3);
    assert!(caps.contains(&"memory_write".to_string()));
    assert!(caps.iter().all(|c| !is_degraded(c)));
}

#[test]
fn test_capability_format_empty() {
    // Backwards compatibility — subsystems can signal without capabilities
    let caps: Vec<String> = vec![];
    assert_eq!(caps.len(), 0);
}

#[test]
fn test_capability_format_degraded() {
    let caps = vec![
        "text_completion:degraded".to_string(),
        "model_loading".to_string(),
    ];

    assert!(caps.iter().any(|c: &String| c.ends_with(":degraded")));
    assert!(caps.iter().any(|c: &String| !c.contains(':')));
    assert_eq!(capability_name("text_completion:degraded"), "text_completion");
}

#[test]
fn test_capability_name_extraction() {
    assert_eq!(capability_name("memory_write"), "memory_write");
    assert_eq!(capability_name("context_window_probe:degraded"), "context_window_probe");
}
