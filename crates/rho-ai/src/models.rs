use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Api
// ---------------------------------------------------------------------------

/// Which API protocol a model uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
	AnthropicMessages,
	OpenAICompletions,
	OpenAIResponses,
}

// ---------------------------------------------------------------------------
// ModelCost
// ---------------------------------------------------------------------------

/// Per-million-token pricing for a model.
#[derive(Debug, Clone, Default)]
pub struct ModelCost {
	pub input_per_mtok:       f64,
	pub output_per_mtok:      f64,
	pub cache_read_per_mtok:  f64,
	pub cache_write_per_mtok: f64,
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Metadata for a single LLM model.
#[derive(Debug, Clone)]
pub struct Model {
	pub id:              String,
	pub name:            String,
	pub provider:        String,
	pub api:             Api,
	pub base_url:        String,
	pub reasoning:       bool,
	pub supports_images: bool,
	pub context_window:  u32,
	pub max_tokens:      u32,
	pub cost:            ModelCost,
}

// ---------------------------------------------------------------------------
// ModelRegistry
// ---------------------------------------------------------------------------

/// A registry of known models, keyed by `(provider, model_id)`.
pub struct ModelRegistry {
	inner: HashMap<String, HashMap<String, Model>>,
}

impl ModelRegistry {
	/// Create an empty registry.
	pub fn new() -> Self {
		Self { inner: HashMap::new() }
	}

	/// Register a model. The provider is taken from `model.provider`.
	pub fn register(&mut self, model: Model) {
		self
			.inner
			.entry(model.provider.clone())
			.or_default()
			.insert(model.id.clone(), model);
	}

	/// Look up a model by provider and model id.
	pub fn get(&self, provider: &str, model_id: &str) -> Option<&Model> {
		self.inner.get(provider)?.get(model_id)
	}

	/// Return all models registered for the given provider.
	pub fn models_for_provider(&self, provider: &str) -> Vec<&Model> {
		self
			.inner
			.get(provider)
			.map(|m| m.values().collect())
			.unwrap_or_default()
	}

	/// Return a list of all known provider names.
	pub fn providers(&self) -> Vec<&str> {
		self.inner.keys().map(|s| s.as_str()).collect()
	}
}

impl Default for ModelRegistry {
	fn default() -> Self {
		Self::new()
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	fn test_model(provider: &str, id: &str, api: Api) -> Model {
		Model {
			id: id.into(),
			name: id.into(),
			provider: provider.into(),
			api,
			base_url: "https://api.example.com".into(),
			reasoning: false,
			supports_images: true,
			context_window: 200000,
			max_tokens: 8192,
			cost: ModelCost::default(),
		}
	}

	#[test]
	fn register_and_get() {
		let mut reg = ModelRegistry::new();
		reg.register(test_model("anthropic", "claude-sonnet-4-5-20250929", Api::AnthropicMessages));
		let m = reg.get("anthropic", "claude-sonnet-4-5-20250929");
		assert!(m.is_some());
		assert_eq!(m.unwrap().id, "claude-sonnet-4-5-20250929");
	}

	#[test]
	fn get_nonexistent() {
		let reg = ModelRegistry::new();
		assert!(reg.get("anthropic", "nonexistent").is_none());
	}

	#[test]
	fn models_for_provider() {
		let mut reg = ModelRegistry::new();
		reg.register(test_model("anthropic", "claude-sonnet-4-5-20250929", Api::AnthropicMessages));
		reg.register(test_model("anthropic", "claude-opus-4-6-20250514", Api::AnthropicMessages));
		reg.register(test_model("openai", "gpt-4o", Api::OpenAICompletions));
		assert_eq!(reg.models_for_provider("anthropic").len(), 2);
		assert_eq!(reg.models_for_provider("openai").len(), 1);
	}

	#[test]
	fn providers_list() {
		let mut reg = ModelRegistry::new();
		reg.register(test_model("anthropic", "m1", Api::AnthropicMessages));
		reg.register(test_model("openai", "m2", Api::OpenAICompletions));
		let providers = reg.providers();
		assert!(providers.contains(&"anthropic"));
		assert!(providers.contains(&"openai"));
	}

	#[test]
	fn api_enum_variants() {
		assert!(matches!(Api::AnthropicMessages, Api::AnthropicMessages));
		assert!(matches!(Api::OpenAICompletions, Api::OpenAICompletions));
		assert!(matches!(Api::OpenAIResponses, Api::OpenAIResponses));
	}
}
