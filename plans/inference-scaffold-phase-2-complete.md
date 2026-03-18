## Phase 2 Complete: error.rs + config.rs

Defined `InferenceError` (13 variants, thiserror) with exhaustive `From<InferenceError> for tonic::Status` mapping. Config loads from TOML with post-load validation. All 8 tests pass.

**Files created/changed:**
- inference/src/error.rs (new)
- inference/src/config.rs (new)
- inference/config/inference.toml (new)
- inference/src/main.rs (added mod declarations)
- inference/Cargo.toml (added tempfile dev-dependency)

**Functions created/changed:**
- `InferenceError` enum with 13 variants
- `From<InferenceError> for tonic::Status` exhaustive mapping
- `Config::load(path: &Path) -> Result<Self, InferenceError>` with validation

**Tests created/changed:**
- test_error_to_status_model_not_found
- test_error_to_status_queue_full
- test_error_to_status_timeout
- test_error_to_status_oom
- test_config_load_valid
- test_config_validation_zero_vram
- test_config_validation_zero_queue
- test_config_validation_missing_model

**Review Status:** APPROVED with minor recommendations applied (unwrap justification comments added, gpu_layers=0 allowed for CPU-only mode)

**Git Commit Message:**
```
feat(inference): add error types and config loading

- Define InferenceError with 13 variants via thiserror
- Implement From<InferenceError> for tonic::Status (exhaustive mapping)
- Add Config struct with TOML deserialization and post-load validation
- Create default inference.toml with all required sections
- 8 tests covering error-to-status mapping and config validation
```
