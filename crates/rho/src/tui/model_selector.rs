//! Model selector — builds [`FilterableSelectItem`]s from the model registry
//! for the interactive model picker with provider tabs.

use std::collections::BTreeSet;

use rho_tui::components::{FilterableSelectItem, tab_bar::Tab};

use crate::settings::Settings;

/// Build a sorted list of [`FilterableSelectItem`]s and provider [`Tab`]s
/// representing every registered model.
///
/// Returns a tuple of:
/// 1. **Tabs:** `[All, Anthropic, OpenAI, ...]` — one per provider, sorted,
///    with "All" first.
/// 2. **Items:** Sorted so that role-assigned models (default, fast, thinking)
///    appear first, followed by all others alphabetically by provider then id.
///    The currently active model is marked with `*`.
pub fn build_model_items(
	registry: &rho_ai::ModelRegistry,
	settings: &Settings,
	current_model_id: &str,
) -> (Vec<Tab>, Vec<FilterableSelectItem>) {
	let mut entries: Vec<ModelEntry> = Vec::new();
	let mut providers = BTreeSet::new();

	for provider in registry.providers() {
		providers.insert(provider.to_owned());
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

	// Sort: role-assigned first (default < fast < thinking), then alphabetically.
	entries.sort_by(|a, b| {
		let a_priority = role_priority(a.role);
		let b_priority = role_priority(b.role);
		a_priority
			.cmp(&b_priority)
			.then_with(|| a.provider.cmp(&b.provider))
			.then_with(|| a.id.cmp(&b.id))
	});

	// Build tabs: "All" first, then one per provider (sorted).
	let mut tabs = vec![Tab::new("all", "All")];
	for provider in &providers {
		// Capitalize first letter for display.
		let label = capitalize(provider);
		tabs.push(Tab::new(provider, &label));
	}

	let items = entries
		.into_iter()
		.map(|e| e.into_filterable_item())
		.collect();

	(tabs, items)
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

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
	let mut chars = s.chars();
	match chars.next() {
		None => String::new(),
		Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
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
	fn into_filterable_item(self) -> FilterableSelectItem {
		// Label: short — "model-name *" (fits in SelectList's 30-char label column).
		let mut label = self.name.clone();
		if self.is_current {
			label.push_str(" *");
		}

		// Description: "provider  200K  [ROLE]  capabilities"
		let ctx = format_context(self.context);
		let mut parts: Vec<String> = vec![self.provider.clone(), ctx];
		if let Some(role) = &self.role {
			let badge = match role {
				Role::Default => "[DEFAULT]",
				Role::Fast => "[FAST]",
				Role::Thinking => "[THINKING]",
			};
			parts.push(badge.to_owned());
		}
		if self.reasoning {
			parts.push("reasoning".to_owned());
		}
		if self.images {
			parts.push("images".to_owned());
		}
		let desc = parts.join("  ");

		// Value: "provider/model_id" — used by resolve_model.
		let value = format!("{}/{}", self.provider, self.id);

		FilterableSelectItem { value, label, description: Some(desc), tab_id: self.provider }
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

	fn make_registry() -> rho_ai::ModelRegistry {
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
		registry
	}

	#[test]
	fn build_items_sorts_roles_first() {
		let registry = make_registry();
		let settings = Settings::default();
		let (tabs, items) = build_model_items(&registry, &settings, "claude-sonnet-4-5-20250929");

		// Tabs: All, Anthropic, OpenAI
		assert_eq!(tabs.len(), 3);
		assert_eq!(tabs[0].id, "all");
		assert_eq!(tabs[1].id, "anthropic");
		assert_eq!(tabs[2].id, "openai");

		// Role-assigned models should come first: DEFAULT, FAST, THINKING.
		// Badges are in description, current marker (*) is in label.
		assert!(
			items[0]
				.description
				.as_deref()
				.unwrap()
				.contains("[DEFAULT]")
		);
		assert!(items[0].label.contains('*')); // current model
		assert!(items[1].description.as_deref().unwrap().contains("[FAST]"));
		assert!(
			items[2]
				.description
				.as_deref()
				.unwrap()
				.contains("[THINKING]")
		);
		// Non-role model last.
		assert!(items[3].label.contains("Other Model"));
		assert!(!items[3].label.contains('*'));
	}

	#[test]
	fn build_items_has_correct_tab_ids() {
		let registry = make_registry();
		let settings = Settings::default();
		let (_, items) = build_model_items(&registry, &settings, "claude-sonnet-4-5-20250929");

		// Anthropic models should have tab_id "anthropic"
		assert_eq!(items[0].tab_id, "anthropic");
		assert_eq!(items[1].tab_id, "anthropic");
		assert_eq!(items[2].tab_id, "anthropic");
		// OpenAI model should have tab_id "openai"
		assert_eq!(items[3].tab_id, "openai");
	}

	#[test]
	fn capitalize_works() {
		assert_eq!(capitalize("anthropic"), "Anthropic");
		assert_eq!(capitalize("openai"), "Openai");
		assert_eq!(capitalize(""), "");
	}
}
