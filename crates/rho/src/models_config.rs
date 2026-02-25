//! Custom provider and model definitions from `~/.rho/models.toml`.
//!
//! Also handles resolving model names to concrete `rho_ai::Model` instances,
//! including role-based lookup ("default", "smol", "slow").

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::settings::Settings;

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

/// Top-level structure of `~/.rho/models.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelsConfig {
	#[serde(default)]
	pub providers: HashMap<String, ProviderConfig>,
}

/// A custom provider definition.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
	pub base_url: String,
	/// Env var name or literal API key.
	pub api_key:  Option<String>,
	/// `"openai-completions"`, `"openai-responses"`, or `"anthropic-messages"`.
	pub api:      String,
	/// Authentication mode.
	#[serde(default)]
	pub auth:     AuthMode,
	/// Extra headers to send with every request.
	#[serde(default)]
	pub headers:  HashMap<String, String>,
	/// Models available from this provider.
	#[serde(default)]
	pub models:   Vec<ModelDef>,
}

/// Authentication mode for a provider.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthMode {
	#[default]
	ApiKey,
	None,
}

/// A model definition within a custom provider.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelDef {
	pub id:              String,
	pub name:            Option<String>,
	#[serde(default)]
	pub reasoning:       bool,
	#[serde(default = "default_true")]
	pub supports_images: bool,
	#[serde(default = "default_context_window")]
	pub context_window:  u32,
	#[serde(default = "default_max_tokens")]
	pub max_tokens:      u32,
	pub cost:            Option<CostDef>,
}

/// Custom cost definition.
#[derive(Debug, Clone, Deserialize)]
pub struct CostDef {
	#[serde(default)]
	pub input:       f64,
	#[serde(default)]
	pub output:      f64,
	#[serde(default)]
	pub cache_read:  f64,
	#[serde(default)]
	pub cache_write: f64,
}

const fn default_true() -> bool {
	true
}
const fn default_context_window() -> u32 {
	128_000
}
const fn default_max_tokens() -> u32 {
	8192
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Path to the models config file (`~/.rho/models.toml`).
fn models_config_path() -> Option<PathBuf> {
	dirs::home_dir().map(|h| h.join(".rho").join("models.toml"))
}

/// Load `~/.rho/models.toml`. Returns an empty config if the file doesn't exist.
pub fn load_models_config() -> Result<ModelsConfig> {
	let Some(path) = models_config_path() else {
		return Ok(ModelsConfig::default());
	};
	if !path.exists() {
		return Ok(ModelsConfig::default());
	}
	let contents = std::fs::read_to_string(&path)?;
	let config: ModelsConfig = toml::from_str(&contents)?;
	Ok(config)
}

/// If the raw string looks like an env var name (all uppercase + underscores +
/// digits), try to resolve it from the environment. Otherwise return as literal.
fn resolve_provider_api_key(raw: &str) -> Option<String> {
	let is_env_var_name = !raw.is_empty()
		&& raw
			.chars()
			.all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
	if is_env_var_name {
		std::env::var(raw).ok()
	} else {
		Some(raw.to_owned())
	}
}

// ---------------------------------------------------------------------------
// Built-in Anthropic models
// ---------------------------------------------------------------------------

/// Register the built-in Anthropic models into the registry.
fn register_builtin_anthropic(registry: &mut rho_ai::ModelRegistry, base_url: &str) {
	let models = [
		rho_ai::Model {
			id:              "claude-sonnet-4-5-20250929".to_owned(),
			name:            "Claude Sonnet 4.5".to_owned(),
			provider:        "anthropic".to_owned(),
			api:             rho_ai::Api::AnthropicMessages,
			base_url:        base_url.to_owned(),
			reasoning:       false,
			supports_images: true,
			context_window:  200_000,
			max_tokens:      8192,
			cost:            rho_ai::ModelCost {
				input_per_mtok:       3.0,
				output_per_mtok:      15.0,
				cache_read_per_mtok:  0.3,
				cache_write_per_mtok: 3.75,
			},
		},
		rho_ai::Model {
			id:              "claude-haiku-4-5-20251001".to_owned(),
			name:            "Claude Haiku 4.5".to_owned(),
			provider:        "anthropic".to_owned(),
			api:             rho_ai::Api::AnthropicMessages,
			base_url:        base_url.to_owned(),
			reasoning:       false,
			supports_images: true,
			context_window:  200_000,
			max_tokens:      8192,
			cost:            rho_ai::ModelCost {
				input_per_mtok:       0.8,
				output_per_mtok:      4.0,
				cache_read_per_mtok:  0.08,
				cache_write_per_mtok: 1.0,
			},
		},
		rho_ai::Model {
			id:              "claude-opus-4-6".to_owned(),
			name:            "Claude Opus 4.6".to_owned(),
			provider:        "anthropic".to_owned(),
			api:             rho_ai::Api::AnthropicMessages,
			base_url:        base_url.to_owned(),
			reasoning:       true,
			supports_images: true,
			context_window:  200_000,
			max_tokens:      16_384,
			cost:            rho_ai::ModelCost {
				input_per_mtok:       15.0,
				output_per_mtok:      75.0,
				cache_read_per_mtok:  1.5,
				cache_write_per_mtok: 18.75,
			},
		},
	];
	for model in models {
		registry.register(model);
	}
}

// ---------------------------------------------------------------------------
// Registry building
// ---------------------------------------------------------------------------

/// Parse an API string ("openai-completions", "openai-responses",
/// "anthropic-messages") into the `rho_ai::Api` enum.
fn parse_api(api: &str) -> Option<rho_ai::Api> {
	match api {
		"openai-completions" => Some(rho_ai::Api::OpenAICompletions),
		"openai-responses" => Some(rho_ai::Api::OpenAIResponses),
		"anthropic-messages" => Some(rho_ai::Api::AnthropicMessages),
		_ => None,
	}
}

/// Build a `ModelRegistry` with built-in Anthropic models and any custom
/// models defined in `models.toml`.
pub fn build_registry(config: &ModelsConfig, settings: &Settings) -> rho_ai::ModelRegistry {
	let mut registry = rho_ai::ModelRegistry::new();

	// Register built-in Anthropic models.
	register_builtin_anthropic(&mut registry, &settings.base_url);

	// Register custom providers.
	for (provider_name, provider) in &config.providers {
		let Some(api) = parse_api(&provider.api) else {
			continue;
		};
		for model_def in &provider.models {
			let cost = model_def.cost.as_ref().map_or_else(rho_ai::ModelCost::default, |c| {
				rho_ai::ModelCost {
					input_per_mtok:       c.input,
					output_per_mtok:      c.output,
					cache_read_per_mtok:  c.cache_read,
					cache_write_per_mtok: c.cache_write,
				}
			});
			registry.register(rho_ai::Model {
				id:              model_def.id.clone(),
				name:            model_def.name.clone().unwrap_or_else(|| model_def.id.clone()),
				provider:        provider_name.clone(),
				api,
				base_url:        provider.base_url.clone(),
				reasoning:       model_def.reasoning,
				supports_images: model_def.supports_images,
				context_window:  model_def.context_window,
				max_tokens:      model_def.max_tokens,
				cost,
			});
		}
	}

	registry
}

// ---------------------------------------------------------------------------
// Model resolution
// ---------------------------------------------------------------------------

/// A resolved model with its API key.
#[derive(Debug)]
pub struct ResolvedModel {
	pub model:   rho_ai::Model,
	pub api_key: String,
}

/// Resolve a model name to a concrete `Model` + API key.
///
/// Resolution order:
/// 1. Role names: `"default"` → `settings.model.default`, `"smol"` →
///    `settings.model.smol`, `"slow"` → `settings.model.slow`
/// 2. `"provider/model_id"` format
/// 3. Bare model ID — search all providers
pub fn resolve_model(
	name: &str,
	registry: &rho_ai::ModelRegistry,
	settings: &Settings,
	models_config: &ModelsConfig,
) -> Result<ResolvedModel> {
	// Step 1: Resolve role aliases.
	let effective_name = match name {
		"default" => &settings.model.default,
		"smol" => &settings.model.smol,
		"slow" => &settings.model.slow,
		other => other,
	};

	// Step 2: Try "provider/model_id" format.
	if let Some((provider, model_id)) = effective_name.split_once('/') {
		if let Some(model) = registry.get(provider, model_id) {
			let api_key = resolve_model_api_key(provider, models_config, settings);
			return Ok(ResolvedModel { model: model.clone(), api_key });
		}
		bail!("Model '{effective_name}' not found in registry");
	}

	// Step 3: Search across all providers.
	for provider_name in registry.providers() {
		if let Some(model) = registry.get(provider_name, effective_name) {
			let api_key = resolve_model_api_key(provider_name, models_config, settings);
			return Ok(ResolvedModel { model: model.clone(), api_key });
		}
	}

	bail!("Model '{effective_name}' not found. Use /model to see available models.")
}

/// Determine the API key for a given provider.
///
/// Custom providers may specify their own key (env var or literal); for
/// "anthropic" we fall back to `settings.api_key`.
fn resolve_model_api_key(
	provider: &str,
	models_config: &ModelsConfig,
	settings: &Settings,
) -> String {
	if let Some(resolved) = models_config
		.providers
		.get(provider)
		.and_then(|cfg| cfg.api_key.as_deref())
		.and_then(resolve_provider_api_key)
	{
		return resolved;
	}
	// Fall back to the default API key (works for anthropic or any provider
	// that shares the same key).
	settings.api_key.clone()
}

/// Return a list of all available model IDs (for display / autocomplete).
pub fn available_models(registry: &rho_ai::ModelRegistry) -> Vec<String> {
	let mut ids: Vec<String> = Vec::new();
	for provider in registry.providers() {
		for model in registry.models_for_provider(provider) {
			ids.push(model.id.clone());
		}
	}
	ids.sort();
	ids
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	fn test_settings() -> Settings {
		Settings {
			api_key:  "test-key".to_owned(),
			base_url: "https://api.anthropic.com".to_owned(),
			..Settings::default()
		}
	}

	#[test]
	fn build_registry_includes_builtins() {
		let config = ModelsConfig::default();
		let settings = test_settings();
		let registry = build_registry(&config, &settings);

		assert!(registry.get("anthropic", "claude-sonnet-4-5-20250929").is_some());
		assert!(registry.get("anthropic", "claude-haiku-4-5-20251001").is_some());
		assert!(registry.get("anthropic", "claude-opus-4-6").is_some());
	}

	#[test]
	fn build_registry_with_custom_provider() {
		let mut providers = HashMap::new();
		providers.insert(
			"ollama".to_owned(),
			ProviderConfig {
				base_url: "http://localhost:11434".to_owned(),
				api_key:  None,
				api:      "openai-completions".to_owned(),
				auth:     AuthMode::None,
				headers:  HashMap::new(),
				models:   vec![ModelDef {
					id:              "llama3".to_owned(),
					name:            Some("Llama 3".to_owned()),
					reasoning:       false,
					supports_images: false,
					context_window:  8192,
					max_tokens:      4096,
					cost:            None,
				}],
			},
		);
		let config = ModelsConfig { providers };
		let settings = test_settings();
		let registry = build_registry(&config, &settings);

		assert!(registry.get("ollama", "llama3").is_some());
		let model = registry.get("ollama", "llama3").unwrap();
		assert_eq!(model.name, "Llama 3");
		assert_eq!(model.context_window, 8192);
	}

	#[test]
	fn resolve_model_by_role() {
		let config = ModelsConfig::default();
		let settings = test_settings();
		let registry = build_registry(&config, &settings);

		let resolved = resolve_model("default", &registry, &settings, &config).unwrap();
		assert_eq!(resolved.model.id, "claude-sonnet-4-5-20250929");

		let resolved = resolve_model("smol", &registry, &settings, &config).unwrap();
		assert_eq!(resolved.model.id, "claude-haiku-4-5-20251001");

		let resolved = resolve_model("slow", &registry, &settings, &config).unwrap();
		assert_eq!(resolved.model.id, "claude-opus-4-6");
	}

	#[test]
	fn resolve_model_by_id() {
		let config = ModelsConfig::default();
		let settings = test_settings();
		let registry = build_registry(&config, &settings);

		let resolved =
			resolve_model("claude-opus-4-6", &registry, &settings, &config).unwrap();
		assert_eq!(resolved.model.id, "claude-opus-4-6");
	}

	#[test]
	fn resolve_model_by_provider_slash_id() {
		let config = ModelsConfig::default();
		let settings = test_settings();
		let registry = build_registry(&config, &settings);

		let resolved = resolve_model(
			"anthropic/claude-sonnet-4-5-20250929",
			&registry,
			&settings,
			&config,
		)
		.unwrap();
		assert_eq!(resolved.model.id, "claude-sonnet-4-5-20250929");
	}

	#[test]
	fn resolve_model_unknown_returns_error() {
		let config = ModelsConfig::default();
		let settings = test_settings();
		let registry = build_registry(&config, &settings);

		let result = resolve_model("some-unknown-model", &registry, &settings, &config);
		assert!(result.is_err());
		let err = result.unwrap_err().to_string();
		assert!(err.contains("not found"), "expected 'not found' in error: {err}");
	}

	#[test]
	fn available_models_lists_all() {
		let config = ModelsConfig::default();
		let settings = test_settings();
		let registry = build_registry(&config, &settings);
		let models = available_models(&registry);
		assert!(models.contains(&"claude-sonnet-4-5-20250929".to_owned()));
		assert!(models.contains(&"claude-haiku-4-5-20251001".to_owned()));
		assert!(models.contains(&"claude-opus-4-6".to_owned()));
	}

	#[test]
	fn resolve_provider_api_key_env_var() {
		// A string that looks like an env var name but doesn't exist returns None.
		assert!(resolve_provider_api_key("NONEXISTENT_KEY_12345").is_none());
	}

	#[test]
	fn resolve_provider_api_key_literal() {
		assert_eq!(
			resolve_provider_api_key("sk-literal-key"),
			Some("sk-literal-key".to_owned())
		);
	}

	#[test]
	fn parse_api_variants() {
		assert_eq!(parse_api("openai-completions"), Some(rho_ai::Api::OpenAICompletions));
		assert_eq!(parse_api("openai-responses"), Some(rho_ai::Api::OpenAIResponses));
		assert_eq!(parse_api("anthropic-messages"), Some(rho_ai::Api::AnthropicMessages));
		assert_eq!(parse_api("unknown"), None);
	}
}
