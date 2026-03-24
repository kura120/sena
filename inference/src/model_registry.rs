use crate::error::InferenceError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Load state for a model in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelLoadState {
    Unloaded,
    Loading,
    Ready,
    Failed(String),
    Switching,
}

/// Metadata for a single model tracked by the registry.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub model_id: String,
    pub model_path: String,
    pub state: ModelLoadState,
    pub vram_estimate_mb: u32,
    pub last_used: Instant,
    pub display_name: String,
}

/// Thread-safe model registry. Inner state protected by RwLock.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    inner: Arc<RwLock<RegistryInner>>,
}

#[derive(Debug)]
struct RegistryInner {
    models: HashMap<String, ModelEntry>,
    active_model_id: Option<String>,
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRegistry {
    /// Create a new empty model registry.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner {
                models: HashMap::new(),
                active_model_id: None,
            })),
        }
    }

    /// Register a new model with the given metadata. Initial state is `Unloaded`.
    pub async fn register(
        &self,
        model_id: String,
        model_path: String,
        vram_estimate_mb: u32,
        display_name: String,
    ) -> Result<(), InferenceError> {
        let entry = ModelEntry {
            model_id: model_id.clone(),
            model_path,
            state: ModelLoadState::Unloaded,
            vram_estimate_mb,
            last_used: Instant::now(),
            display_name,
        };

        let mut guard = self.inner.write().await;
        guard.models.insert(model_id, entry);
        Ok(())
    }

    /// Update the load state for a model. Returns `ModelNotFound` if the model is not registered.
    pub async fn set_state(
        &self,
        model_id: &str,
        state: ModelLoadState,
    ) -> Result<(), InferenceError> {
        let mut guard = self.inner.write().await;
        let entry =
            guard
                .models
                .get_mut(model_id)
                .ok_or_else(|| InferenceError::ModelNotFound {
                    model_id: model_id.to_string(),
                })?;
        entry.state = state;
        Ok(())
    }

    /// Get the current load state for a model. Returns `ModelNotFound` if the model is not registered.
    pub async fn get_state(&self, model_id: &str) -> Result<ModelLoadState, InferenceError> {
        let guard = self.inner.read().await;
        let entry = guard
            .models
            .get(model_id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                model_id: model_id.to_string(),
            })?;
        Ok(entry.state.clone())
    }

    /// Set the active model. Returns `ModelNotFound` if the model is not registered.
    pub async fn set_active(&self, model_id: &str) -> Result<(), InferenceError> {
        let mut guard = self.inner.write().await;
        if !guard.models.contains_key(model_id) {
            return Err(InferenceError::ModelNotFound {
                model_id: model_id.to_string(),
            });
        }
        guard.active_model_id = Some(model_id.to_string());
        Ok(())
    }

    /// Get the current active model ID, if any.
    pub async fn active_model_id(&self) -> Option<String> {
        let guard = self.inner.read().await;
        guard.active_model_id.clone()
    }

    /// Find the least recently used `Ready` model that is not currently active.
    /// Returns `None` if no such model exists.
    pub async fn lru_eviction_candidate(&self) -> Option<String> {
        let guard = self.inner.read().await;
        guard
            .models
            .values()
            .filter(|entry| {
                entry.state == ModelLoadState::Ready
                    && Some(&entry.model_id) != guard.active_model_id.as_ref()
            })
            .min_by_key(|entry| entry.last_used)
            .map(|entry| entry.model_id.clone())
    }

    /// Calculate the total VRAM allocated by all models in `Ready` or `Loading` state.
    pub async fn total_vram_allocated_mb(&self) -> u32 {
        let guard = self.inner.read().await;
        guard
            .models
            .values()
            .filter(|entry| {
                matches!(
                    entry.state,
                    ModelLoadState::Ready | ModelLoadState::Loading | ModelLoadState::Switching
                )
            })
            .map(|entry| entry.vram_estimate_mb)
            .sum()
    }

    /// Update the `last_used` timestamp for a model. Returns `ModelNotFound` if the model is not registered.
    pub async fn touch(&self, model_id: &str) -> Result<(), InferenceError> {
        let mut guard = self.inner.write().await;
        let entry =
            guard
                .models
                .get_mut(model_id)
                .ok_or_else(|| InferenceError::ModelNotFound {
                    model_id: model_id.to_string(),
                })?;
        entry.last_used = Instant::now();
        Ok(())
    }

    /// Get the display name for a model. Returns `ModelNotFound` if the model is not registered.
    pub async fn get_display_name(&self, model_id: &str) -> Result<String, InferenceError> {
        let guard = self.inner.read().await;
        let entry = guard
            .models
            .get(model_id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                model_id: model_id.to_string(),
            })?;
        Ok(entry.display_name.clone())
    }

    /// Remove a model entry from the registry entirely.
    /// Frees its VRAM budget allocation and clears active_model_id if this was the active model.
    pub async fn remove(&self, model_id: &str) -> Result<(), InferenceError> {
        let mut guard = self.inner.write().await;
        if guard.models.remove(model_id).is_none() {
            return Err(InferenceError::ModelNotFound {
                model_id: model_id.to_string(),
            });
        }
        // If this was the active model, clear active_model_id
        if guard.active_model_id.as_deref() == Some(model_id) {
            guard.active_model_id = None;
        }
        Ok(())
    }

    /// List all registered models and their metadata.
    pub async fn list_all(&self) -> Vec<ModelEntry> {
        let guard = self.inner.read().await;
        guard.models.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_model() {
        let registry = ModelRegistry::new();
        registry
            .register(
                "model-a".into(),
                "/path/a.gguf".into(),
                2048,
                "Model A".into(),
            )
            .await
            .expect("test: register should succeed");
        let state = registry
            .get_state("model-a")
            .await
            .expect("test: get_state should succeed");
        assert_eq!(state, ModelLoadState::Unloaded);
    }

    #[tokio::test]
    async fn test_set_state_transitions() {
        let registry = ModelRegistry::new();
        registry
            .register("m1".into(), "/p".into(), 1024, "M1".into())
            .await
            .expect("test: register");

        registry
            .set_state("m1", ModelLoadState::Loading)
            .await
            .expect("test: set loading");
        assert_eq!(
            registry.get_state("m1").await.expect("test: get"),
            ModelLoadState::Loading
        );

        registry
            .set_state("m1", ModelLoadState::Ready)
            .await
            .expect("test: set ready");
        assert_eq!(
            registry.get_state("m1").await.expect("test: get"),
            ModelLoadState::Ready
        );

        registry
            .set_state("m1", ModelLoadState::Unloaded)
            .await
            .expect("test: set unloaded");
        assert_eq!(
            registry.get_state("m1").await.expect("test: get"),
            ModelLoadState::Unloaded
        );
    }

    #[tokio::test]
    async fn test_set_active_model() {
        let registry = ModelRegistry::new();
        registry
            .register("a".into(), "/a".into(), 512, "A".into())
            .await
            .expect("test: register a");
        registry
            .register("b".into(), "/b".into(), 512, "B".into())
            .await
            .expect("test: register b");
        registry.set_active("a").await.expect("test: set active");
        assert_eq!(registry.active_model_id().await, Some("a".to_string()));
    }

    #[tokio::test]
    async fn test_set_active_nonexistent() {
        let registry = ModelRegistry::new();
        let result = registry.set_active("ghost").await;
        assert!(result.is_err());
        // Should be ModelNotFound
        match result.expect_err("test: should be error") {
            InferenceError::ModelNotFound { model_id } => assert_eq!(model_id, "ghost"),
            other => panic!("Expected ModelNotFound, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_lru_candidate_returns_least_recently_used() {
        let registry = ModelRegistry::new();
        registry
            .register("old".into(), "/old".into(), 1024, "Old".into())
            .await
            .expect("test: register");
        registry
            .register("new".into(), "/new".into(), 1024, "New".into())
            .await
            .expect("test: register");

        registry
            .set_state("old", ModelLoadState::Ready)
            .await
            .expect("test: set ready");
        registry
            .set_state("new", ModelLoadState::Ready)
            .await
            .expect("test: set ready");

        // Touch "new" to make it more recently used
        registry.touch("new").await.expect("test: touch");

        // Set "new" as active — LRU should return "old" (not active, least recently used)
        registry.set_active("new").await.expect("test: set active");

        let candidate = registry.lru_eviction_candidate().await;
        assert_eq!(candidate, Some("old".to_string()));
    }

    #[tokio::test]
    async fn test_lru_candidate_empty() {
        let registry = ModelRegistry::new();
        assert_eq!(registry.lru_eviction_candidate().await, None);
    }

    #[tokio::test]
    async fn test_total_vram_allocated() {
        let registry = ModelRegistry::new();
        registry
            .register("a".into(), "/a".into(), 1024, "A".into())
            .await
            .expect("test: register");
        registry
            .register("b".into(), "/b".into(), 2048, "B".into())
            .await
            .expect("test: register");

        // Only Ready/Loading models count
        registry
            .set_state("a", ModelLoadState::Ready)
            .await
            .expect("test: set");
        registry
            .set_state("b", ModelLoadState::Ready)
            .await
            .expect("test: set");

        assert_eq!(registry.total_vram_allocated_mb().await, 3072);
    }

    #[tokio::test]
    async fn test_remove_model() {
        let registry = ModelRegistry::new();
        registry
            .register(
                "model-a".into(),
                "/path/a.gguf".into(),
                2048,
                "Model A".into(),
            )
            .await
            .expect("test: register should succeed");

        // Verify it exists
        let state = registry
            .get_state("model-a")
            .await
            .expect("test: get_state should succeed");
        assert_eq!(state, ModelLoadState::Unloaded);

        // Remove it
        registry
            .remove("model-a")
            .await
            .expect("test: remove should succeed");

        // Verify it's gone
        let result = registry.get_state("model-a").await;
        assert!(result.is_err(), "Expected ModelNotFound after removal");
        match result.expect_err("test: should be error") {
            InferenceError::ModelNotFound { model_id } => assert_eq!(model_id, "model-a"),
            other => panic!("Expected ModelNotFound, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_remove_clears_active() {
        let registry = ModelRegistry::new();
        registry
            .register(
                "active-model".into(),
                "/path/active.gguf".into(),
                1024,
                "Active Model".into(),
            )
            .await
            .expect("test: register should succeed");

        // Set as active
        registry
            .set_active("active-model")
            .await
            .expect("test: set_active should succeed");
        assert_eq!(
            registry.active_model_id().await,
            Some("active-model".to_string())
        );

        // Remove it
        registry
            .remove("active-model")
            .await
            .expect("test: remove should succeed");

        // Verify active_model_id is cleared
        assert_eq!(registry.active_model_id().await, None);
    }

    #[tokio::test]
    async fn test_registry_stores_display_name() {
        let registry = ModelRegistry::new();
        registry
            .register(
                "gemma-2b".into(),
                "/models/gemma-2b.gguf".into(),
                2048,
                "Gemma 2.0 2B Instruct".into(),
            )
            .await
            .expect("test: register should succeed");

        let display_name = registry
            .get_display_name("gemma-2b")
            .await
            .expect("test: get_display_name should succeed");
        assert_eq!(display_name, "Gemma 2.0 2B Instruct");
    }

    #[tokio::test]
    async fn test_get_display_name_not_found() {
        let registry = ModelRegistry::new();
        let result = registry.get_display_name("nonexistent").await;
        assert!(result.is_err());
        match result.expect_err("test: should be error") {
            InferenceError::ModelNotFound { model_id } => assert_eq!(model_id, "nonexistent"),
            other => panic!("Expected ModelNotFound, got: {other}"),
        }
    }
}
