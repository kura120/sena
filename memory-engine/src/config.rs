//! Configuration loading for memory-engine.
//!
//! Deserializes `memory-engine.toml` into typed structs via `serde` + `toml`.
//! Every threshold, timeout, and tunable value used by memory-engine originates
//! from this config — nothing is hardcoded in source.
//!
//! Provides `Config::load(path)` as the single entry point. All fields are
//! required — there are no hidden defaults baked into the binary.

use std::path::Path;

use serde::Deserialize;

use crate::error::{ErrorCode, SenaError, SenaResult};

// ─────────────────────────────────────────────────────────────────────────────
// Top-level config
// ─────────────────────────────────────────────────────────────────────────────

/// Root configuration for memory-engine, mirroring the structure of
/// `config/memory-engine.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub grpc: GrpcConfig,
    pub boot: BootConfig,
    pub store: StorePathsConfig,
    pub tier: TierConfig,
    pub decay: DecayConfig,
    pub queue: QueueConfig,
    pub embedder: EmbedderConfig,
    pub extractor: ExtractorConfig,
    pub logging: LoggingConfig,
}

impl Config {
    /// Load and parse configuration from a TOML file at `path`.
    ///
    /// Fails with `ErrorCode::ConfigLoadFailure` if the file cannot be read
    /// or if deserialization fails. The error message never contains file
    /// content — only the path and the parse error description.
    pub fn load(path: &Path) -> SenaResult<Self> {
        let raw_content = std::fs::read_to_string(path).map_err(|io_error| {
            SenaError::new(
                ErrorCode::ConfigLoadFailure,
                "failed to read memory-engine config file",
            )
            .with_debug_context(format!(
                "path: {}, io error: {}",
                path.display(),
                io_error
            ))
        })?;

        let config: Config = toml::from_str(&raw_content).map_err(|toml_error| {
            SenaError::new(
                ErrorCode::ConfigLoadFailure,
                "failed to parse memory-engine.toml",
            )
            .with_debug_context(format!(
                "path: {}, toml error: {}",
                path.display(),
                toml_error
            ))
        })?;

        Ok(config)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Store paths
// ─────────────────────────────────────────────────────────────────────────────

/// Filesystem paths for ech0's persistent storage backends.
///
/// ech0 uses two separate files: a redb graph database and a usearch vector
/// index. Both paths are relative to the memory-engine process working directory.
#[derive(Debug, Clone, Deserialize)]
pub struct StorePathsConfig {
    /// Path to the redb graph database file (e.g. `data/graph.redb`).
    pub graph_path: String,
    /// Path to the usearch vector index file (e.g. `data/vectors.usearch`).
    pub vector_path: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// gRPC
// ─────────────────────────────────────────────────────────────────────────────

/// Connection settings for daemon-bus and the local MemoryService server.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    /// Address of the daemon-bus gRPC server (e.g. `http://127.0.0.1:50051`).
    pub daemon_bus_address: String,
    /// Address on which memory-engine binds its own MemoryService gRPC server.
    /// Should be "127.0.0.1" for localhost-only access.
    pub listen_address: String,
    /// Port on which memory-engine serves its own MemoryService gRPC server.
    pub listen_port: u16,
    /// Maximum time in milliseconds to wait for the initial daemon-bus connection.
    pub connect_timeout_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Boot
// ─────────────────────────────────────────────────────────────────────────────

/// Boot sequence timing constraints.
#[derive(Debug, Clone, Deserialize)]
pub struct BootConfig {
    /// Maximum wall-clock time in milliseconds for the entire boot sequence
    /// to complete before the process aborts with `ErrorCode::BootTimeout`.
    pub ready_signal_timeout_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tiers
// ─────────────────────────────────────────────────────────────────────────────

/// Container for all tier-specific configurations.
#[derive(Debug, Clone, Deserialize)]
pub struct TierConfig {
    pub short_term: ShortTermTierConfig,
    pub long_term: LongTermTierConfig,
    pub episodic: EpisodicTierConfig,
}

/// Short-term tier — volatile, session-scoped memory.
#[derive(Debug, Clone, Deserialize)]
pub struct ShortTermTierConfig {
    /// Maximum number of entries held in the short-term tier.
    pub max_entries: u32,
}

/// Long-term tier — persistent, promoted from short-term by CTP.
#[derive(Debug, Clone, Deserialize)]
pub struct LongTermTierConfig {
    /// Maximum number of entries in the long-term tier.
    pub max_entries: u32,
}

/// Episodic tier — append-only, never mutated or deleted.
#[derive(Debug, Clone, Deserialize)]
pub struct EpisodicTierConfig {
    /// Maximum number of entries in the episodic tier.
    pub max_entries: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Decay
// ─────────────────────────────────────────────────────────────────────────────

/// Importance decay configuration. Used by ech0's `importance-decay` feature.
#[derive(Debug, Clone, Deserialize)]
pub struct DecayConfig {
    /// Multiplicative decay factor applied per cycle (0.0–1.0).
    pub rate: f32,
    /// Absolute minimum weight. No entry ever decays below this — prevents
    /// zero-weight ghosts that consume storage but never surface in search.
    pub floor: f32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Write queue
// ─────────────────────────────────────────────────────────────────────────────

/// Write queue configuration. All `store.ingest_text()` calls are serialized
/// through the async write queue — these values bound its depth and timing.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    /// Maximum number of pending write operations before `QueueFull` is returned.
    pub max_depth: u32,
    /// Maximum wall-clock time in milliseconds for a single queued operation.
    pub operation_timeout_ms: u64,
    /// Retry policy for transient ech0 errors.
    pub retry: RetryConfig,
}

/// Retry policy for the write queue.
#[derive(Debug, Clone, Deserialize)]
pub struct RetryConfig {
    /// Maximum retry attempts on transient `EchoError` before giving up.
    pub max_attempts: u32,
    /// Delay in milliseconds between retry attempts.
    pub backoff_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Embedder
// ─────────────────────────────────────────────────────────────────────────────

/// Embedder (llama-cpp-rs) configuration for producing vector embeddings.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbedderConfig {
    /// Path to the GGUF model file used for embedding generation.
    pub model_path: String,
    /// Embedding output dimensionality — must match the GGUF model's embedding
    /// output dimension. There is no default; missing field is a `ConfigLoadFailure`.
    pub embedding_dim: usize,
    /// Number of texts to batch in a single embedding call.
    pub batch_size: u32,
    /// Number of GPU layers to offload.
    pub gpu_layers: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Extractor
// ─────────────────────────────────────────────────────────────────────────────

/// Extractor (llama-cpp-rs) configuration for graph/entity extraction.
#[derive(Debug, Clone, Deserialize)]
pub struct ExtractorConfig {
    /// Path to the GGUF model file used for extraction.
    pub model_path: String,
    /// Number of GPU layers to offload.
    pub gpu_layers: u32,
    /// Maximum tokens the extractor may generate per extraction call.
    pub max_tokens: u32,
    /// Temperature for extraction inference. 0.0 for deterministic output.
    pub temperature: f32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Logging
// ─────────────────────────────────────────────────────────────────────────────

/// Logging configuration for the tracing subscriber.
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Tracing filter level (e.g. "info", "debug", "warn").
    pub level: String,
    /// Output format: "pretty" or "json".
    pub format: String,
    /// Threshold in milliseconds above which any memory operation is logged
    /// at warn level regardless of its configured priority.
    pub slow_operation_threshold_ms: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Minimal valid TOML that exercises every config field.
    const VALID_TOML: &str = r#"
[grpc]
daemon_bus_address = "http://127.0.0.1:50051"
listen_address = "127.0.0.1"
listen_port = 50053
connect_timeout_ms = 5000

[boot]
ready_signal_timeout_ms = 15000

[tier.short_term]
max_entries = 256

[tier.long_term]
max_entries = 10000

[tier.episodic]
max_entries = 50000

[decay]
rate = 0.95
floor = 0.05

[queue]
max_depth = 512
operation_timeout_ms = 10000

[queue.retry]
max_attempts = 3
backoff_ms = 500

[embedder]
model_path = "models/embedding.gguf"
embedding_dim = 768
batch_size = 32
gpu_layers = 99

[store]
graph_path = "data/graph.redb"
vector_path = "data/vectors.usearch"

[extractor]
model_path = "models/default.gguf"
gpu_layers = 99
max_tokens = 512
temperature = 0.0

[logging]
level = "info"
format = "pretty"
slow_operation_threshold_ms = 100
"#;

    #[test]
    fn load_valid_config_from_disk() {
        let temp_dir = std::env::temp_dir().join("memory_engine_config_test");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config_path = temp_dir.join("memory-engine.toml");
        let mut file = std::fs::File::create(&config_path).expect("create temp file");
        file.write_all(VALID_TOML.as_bytes())
            .expect("write temp file");

        let config = Config::load(&config_path).expect("config should load");

        assert_eq!(config.grpc.daemon_bus_address, "http://127.0.0.1:50051");
        assert_eq!(config.grpc.listen_address, "127.0.0.1");
        assert_eq!(config.grpc.listen_port, 50053);
        assert_eq!(config.grpc.connect_timeout_ms, 5000);
        assert_eq!(config.boot.ready_signal_timeout_ms, 15000);
        assert_eq!(config.store.graph_path, "data/graph.redb");
        assert_eq!(config.store.vector_path, "data/vectors.usearch");
        assert_eq!(config.tier.short_term.max_entries, 256);
        assert_eq!(config.tier.long_term.max_entries, 10000);
        assert_eq!(config.tier.episodic.max_entries, 50000);
        assert_eq!(config.decay.rate, 0.95);
        assert_eq!(config.decay.floor, 0.05);
        assert_eq!(config.queue.max_depth, 512);
        assert_eq!(config.queue.operation_timeout_ms, 10000);
        assert_eq!(config.queue.retry.max_attempts, 3);
        assert_eq!(config.queue.retry.backoff_ms, 500);
        assert_eq!(config.embedder.model_path, "models/embedding.gguf");
        assert_eq!(config.embedder.embedding_dim, 768);
        assert_eq!(config.embedder.batch_size, 32);
        assert_eq!(config.embedder.gpu_layers, 99);
        assert_eq!(config.extractor.model_path, "models/default.gguf");
        assert_eq!(config.extractor.gpu_layers, 99);
        assert_eq!(config.extractor.max_tokens, 512);
        assert_eq!(config.extractor.temperature, 0.0);
        assert_eq!(config.logging.level, "info");
        assert_eq!(config.logging.format, "pretty");
        assert_eq!(config.logging.slow_operation_threshold_ms, 100);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_missing_file_returns_config_load_failure() {
        let result = Config::load(Path::new("/nonexistent/path/memory-engine.toml"));
        let error = result.expect_err("should fail on missing file");
        assert_eq!(error.code, ErrorCode::ConfigLoadFailure);
        assert!(error.debug_context.is_some());
    }

    #[test]
    fn load_malformed_toml_returns_config_load_failure() {
        let temp_dir = std::env::temp_dir().join("memory_engine_config_bad_test");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config_path = temp_dir.join("memory-engine.toml");
        std::fs::write(&config_path, "this is not valid toml {{{{").expect("write bad file");

        let result = Config::load(&config_path);
        let error = result.expect_err("should fail on malformed toml");
        assert_eq!(error.code, ErrorCode::ConfigLoadFailure);
        assert!(error.debug_context.is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_incomplete_toml_returns_config_load_failure() {
        let temp_dir = std::env::temp_dir().join("memory_engine_config_incomplete_test");
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");

        let config_path = temp_dir.join("memory-engine.toml");
        // Valid TOML but missing required fields
        std::fs::write(&config_path, "[grpc]\ndaemon_bus_address = \"x\"\n")
            .expect("write partial file");

        let result = Config::load(&config_path);
        let error = result.expect_err("should fail on incomplete config");
        assert_eq!(error.code, ErrorCode::ConfigLoadFailure);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
