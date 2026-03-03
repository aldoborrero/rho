//! Model selector — builds [`SelectItem`]s from the model registry for the
//! interactive model picker.

use rho_tui::components::select_list::SelectItem;

use crate::settings::Settings;

/// Build a sorted list of [`SelectItem`]s representing every registered model.
///
/// Items are sorted so that role-assigned models (default, smol, slow) appear
/// first, followed by all others alphabetically by provider then id. The
/// currently active model is marked with `*`.
pub fn build_model_items(
	registry: &rho_ai::ModelRegistry,
	settings: &Settings,
	current_model_id: &str,
) -> Vec<SelectItem> {
	let mut entries: Vec<ModelEntry> = Vec::new();

	for provider in registry.providers() {
		for model in registry.models_for_provider(provider) {
			let role = model_role(&model.id, settings);
			entries.push(ModelEntry {
				provider: model.provider.clone(),
				id: model.id.clone(),
				name: model.name.clone(),
				context: model.context_window,
				reasoning: model.reasoning,
				images: model.supports_images,
				role,
				is_current: model.id == current_model_id,
			});
		}
	}

	// Sort: role-assigned first (default < smol < slow), then alphabetically.
	entries.sort_by(|a, b| {
		let a_priority = role_priority(a.role);
		let b_priority = role_priority(b.role);
		a_priority
			.cmp(&b_priority)
			.then_with(|| a.provider.cmp(&b.provider))
			.then_with(|| a.id.cmp(&b.id))
	});

	entries.into_iter().map(|e| e.into_select_item()).collect()
}

/// Format a context window size into a human-readable string.
///
/// Uses integer arithmetic to avoid floating-point comparison pitfalls.
fn format_context(tokens: u32) -> String {
	if tokens >= 1_000_000 {
		if tokens.is_multiple_of(1_000_000) {
			format!("{}M", tokens / 1_000_000)
		} else {
			// One decimal place: 1_500_000 → "1.5M"
			let tenths = (tokens % 1_000_000) / 100_000;
			format!("{}.{}M", tokens / 1_000_000, tenths)
		}
	} else if tokens >= 1_000 {
		if tokens.is_multiple_of(1_000) {
			format!("{}K", tokens / 1_000)
		} else {
			let tenths = (tokens % 1_000) / 100;
			format!("{}.{}K", tokens / 1_000, tenths)
		}
	} else {
		tokens.to_string()
	}
}

/// Determine which role (if any) a model is assigned to.
fn model_role(model_id: &str, settings: &Settings) -> Option<Role> {
	if model_id == settings.model.default {
		Some(Role::Default)
	} else if model_id == settings.model.smol {
		Some(Role::Fast)
	} else if model_id == settings.model.slow {
		Some(Role::Thinking)
	} else {
		None
	}
}

/// Sort priority for roles — lower is higher priority.
const fn role_priority(role: Option<Role>) -> u8 {
	match role {
		Some(Role::Default) => 0,
		Some(Role::Fast) => 1,
		Some(Role::Thinking) => 2,
		None => 3,
	}
}

// ── Internal types ──────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Role {
	Default,
	Fast,
	Thinking,
}

struct ModelEntry {
	provider:   String,
	id:         String,
	name:       String,
	context:    u32,
	reasoning:  bool,
	images:     bool,
	role:       Option<Role>,
	is_current: bool,
}

impl ModelEntry {
	fn into_select_item(self) -> SelectItem {
		// Label: "model-name  200K  [ROLE] *"
		let ctx = format_context(self.context);
		let mut label = format!("{}  {ctx}", self.name);
		if let Some(role) = &self.role {
			let badge = match role {
				Role::Default => " [DEFAULT]",
				Role::Fast => " [FAST]",
				Role::Thinking => " [THINKING]",
			};
			label.push_str(badge);
		}
		if self.is_current {
			label.push_str(" *");
		}

		// Description: "provider  capabilities"
		let mut caps: Vec<&str> = Vec::new();
		if self.reasoning {
			caps.push("reasoning");
		}
		if self.images {
			caps.push("images");
		}
		let caps_str = if caps.is_empty() {
			String::new()
		} else {
			format!("  {}", caps.join("  "))
		};
		let desc = format!("{}{caps_str}", self.provider);

		// Value: "provider/model_id" — used by resolve_model.
		let value = format!("{}/{}", self.provider, self.id);

		SelectItem::new(&value, &label).with_description(&desc)
	}
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn format_context_thousands() {
		assert_eq!(format_context(200_000), "200K");
		assert_eq!(format_context(128_000), "128K");
		assert_eq!(format_context(8_192), "8.1K");
		assert_eq!(format_context(16_384), "16.3K");
	}

	#[test]
	fn format_context_millions() {
		assert_eq!(format_context(1_000_000), "1M");
		assert_eq!(format_context(2_000_000), "2M");
	}

	#[test]
	fn format_context_small() {
		assert_eq!(format_context(500), "500");
	}

	#[test]
	fn role_detection() {
		let settings = Settings::default();
		assert!(matches!(model_role("claude-sonnet-4-5-20250929", &settings), Some(Role::Default)));
		assert!(matches!(model_role("claude-haiku-4-5-20251001", &settings), Some(Role::Fast)));
		assert!(matches!(model_role("claude-opus-4-6", &settings), Some(Role::Thinking)));
		assert!(model_role("unknown-model", &settings).is_none());
	}

	#[test]
	fn build_items_sorts_roles_first() {
		let mut registry = rho_ai::ModelRegistry::new();
		registry.register(rho_ai::Model {
			id:              "claude-sonnet-4-5-20250929".to_owned(),
			name:            "Claude Sonnet 4.5".to_owned(),
			provider:        "anthropic".to_owned(),
			api:             rho_ai::Api::AnthropicMessages,
			base_url:        String::new(),
			reasoning:       false,
			supports_images: true,
			context_window:  200_000,
			max_tokens:      8_192,
			cost:            rho_ai::ModelCost::default(),
		});
		registry.register(rho_ai::Model {
			id:              "claude-haiku-4-5-20251001".to_owned(),
			name:            "Claude Haiku 4.5".to_owned(),
			provider:        "anthropic".to_owned(),
			api:             rho_ai::Api::AnthropicMessages,
			base_url:        String::new(),
			reasoning:       false,
			supports_images: true,
			context_window:  200_000,
			max_tokens:      8_192,
			cost:            rho_ai::ModelCost::default(),
		});
		registry.register(rho_ai::Model {
			id:              "claude-opus-4-6".to_owned(),
			name:            "Claude Opus 4.6".to_owned(),
			provider:        "anthropic".to_owned(),
			api:             rho_ai::Api::AnthropicMessages,
			base_url:        String::new(),
			reasoning:       true,
			supports_images: true,
			context_window:  200_000,
			max_tokens:      16_384,
			cost:            rho_ai::ModelCost::default(),
		});
		registry.register(rho_ai::Model {
			id:              "some-other-model".to_owned(),
			name:            "Other Model".to_owned(),
			provider:        "openai".to_owned(),
			api:             rho_ai::Api::OpenAICompletions,
			base_url:        String::new(),
			reasoning:       false,
			supports_images: false,
			context_window:  128_000,
			max_tokens:      4_096,
			cost:            rho_ai::ModelCost::default(),
		});

		let settings = Settings::default();
		let items = build_model_items(&registry, &settings, "claude-sonnet-4-5-20250929");

		// Role-assigned models should come first: DEFAULT, FAST, THINKING.
		assert!(items[0].label.contains("[DEFAULT]"));
		assert!(items[0].label.contains("*")); // current model
		assert!(items[1].label.contains("[FAST]"));
		assert!(items[2].label.contains("[THINKING]"));
		// Non-role model last.
		assert!(items[3].label.contains("Other Model"));
		assert!(!items[3].label.contains('*'));
	}
}
