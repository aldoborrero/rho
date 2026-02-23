//! Utility functions for mapping the unified [`ToolChoice`] enum to
//! provider-specific JSON representations.
//!
//! Each provider expects tool-choice in a slightly different shape:
//!
//! | Variant     | Anthropic                         | OpenAI Completions           | OpenAI Responses              |
//! |-------------|-----------------------------------|------------------------------|-------------------------------|
//! | Auto        | `"auto"`                          | `"auto"`                     | `"auto"`                      |
//! | None        | `"none"`                          | `"none"`                     | `"none"`                      |
//! | Any         | `{"type":"any"}`                  | `"required"`                 | `"required"`                  |
//! | Required    | `{"type":"any"}`                  | `"required"`                 | `"required"`                  |
//! | Specific(n) | `{"type":"tool","name":n}`        | `{"type":"function","function":{"name":n}}` | `{"type":"function","name":n}` |

use serde_json::Value;

use crate::types::ToolChoice;

/// Map a unified [`ToolChoice`] to the Anthropic API format.
pub fn to_anthropic(choice: Option<&ToolChoice>) -> Option<Value> {
	let choice = choice?;
	Some(match choice {
		ToolChoice::Auto => serde_json::json!("auto"),
		ToolChoice::None => serde_json::json!("none"),
		ToolChoice::Any | ToolChoice::Required => serde_json::json!({"type": "any"}),
		ToolChoice::Specific { name } => serde_json::json!({"type": "tool", "name": name}),
	})
}

/// Map a unified [`ToolChoice`] to the OpenAI Completions API format.
pub fn to_openai_completions(choice: Option<&ToolChoice>) -> Option<Value> {
	let choice = choice?;
	Some(match choice {
		ToolChoice::Auto => serde_json::json!("auto"),
		ToolChoice::None => serde_json::json!("none"),
		ToolChoice::Any | ToolChoice::Required => serde_json::json!("required"),
		ToolChoice::Specific { name } => {
			serde_json::json!({"type": "function", "function": {"name": name}})
		},
	})
}

/// Map a unified [`ToolChoice`] to the OpenAI Responses API format.
///
/// This differs from Completions in the `Specific` variant: the name lives
/// directly on the object (`{"type":"function","name":...}`) rather than in
/// a nested `"function"` sub-object.
pub fn to_openai_responses(choice: Option<&ToolChoice>) -> Option<Value> {
	let choice = choice?;
	Some(match choice {
		ToolChoice::Auto => serde_json::json!("auto"),
		ToolChoice::None => serde_json::json!("none"),
		ToolChoice::Any | ToolChoice::Required => serde_json::json!("required"),
		ToolChoice::Specific { name } => serde_json::json!({"type": "function", "name": name}),
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::ToolChoice;

	// -----------------------------------------------------------------------
	// Anthropic mapping
	// -----------------------------------------------------------------------

	#[test]
	fn anthropic_auto() {
		assert_eq!(to_anthropic(Some(&ToolChoice::Auto)), Some(serde_json::json!("auto")));
	}

	#[test]
	fn anthropic_none() {
		assert_eq!(to_anthropic(Some(&ToolChoice::None)), Some(serde_json::json!("none")));
	}

	#[test]
	fn anthropic_any() {
		assert_eq!(to_anthropic(Some(&ToolChoice::Any)), Some(serde_json::json!({"type": "any"})));
	}

	#[test]
	fn anthropic_required_maps_to_any() {
		assert_eq!(
			to_anthropic(Some(&ToolChoice::Required)),
			Some(serde_json::json!({"type": "any"}))
		);
	}

	#[test]
	fn anthropic_specific() {
		let tc = ToolChoice::Specific { name: "bash".into() };
		assert_eq!(
			to_anthropic(Some(&tc)),
			Some(serde_json::json!({"type": "tool", "name": "bash"}))
		);
	}

	#[test]
	fn anthropic_none_option() {
		assert_eq!(to_anthropic(None), None);
	}

	// -----------------------------------------------------------------------
	// OpenAI Completions mapping
	// -----------------------------------------------------------------------

	#[test]
	fn openai_completions_auto() {
		assert_eq!(to_openai_completions(Some(&ToolChoice::Auto)), Some(serde_json::json!("auto")));
	}

	#[test]
	fn openai_completions_none() {
		assert_eq!(to_openai_completions(Some(&ToolChoice::None)), Some(serde_json::json!("none")));
	}

	#[test]
	fn openai_completions_required() {
		assert_eq!(
			to_openai_completions(Some(&ToolChoice::Required)),
			Some(serde_json::json!("required"))
		);
	}

	#[test]
	fn openai_completions_any_maps_to_required() {
		assert_eq!(
			to_openai_completions(Some(&ToolChoice::Any)),
			Some(serde_json::json!("required"))
		);
	}

	#[test]
	fn openai_completions_specific() {
		let tc = ToolChoice::Specific { name: "bash".into() };
		let expected = serde_json::json!({"type": "function", "function": {"name": "bash"}});
		assert_eq!(to_openai_completions(Some(&tc)), Some(expected));
	}

	#[test]
	fn openai_completions_none_option() {
		assert_eq!(to_openai_completions(None), None);
	}

	// -----------------------------------------------------------------------
	// OpenAI Responses mapping
	// -----------------------------------------------------------------------

	#[test]
	fn openai_responses_auto() {
		assert_eq!(to_openai_responses(Some(&ToolChoice::Auto)), Some(serde_json::json!("auto")));
	}

	#[test]
	fn openai_responses_none() {
		assert_eq!(to_openai_responses(Some(&ToolChoice::None)), Some(serde_json::json!("none")));
	}

	#[test]
	fn openai_responses_required() {
		assert_eq!(
			to_openai_responses(Some(&ToolChoice::Required)),
			Some(serde_json::json!("required"))
		);
	}

	#[test]
	fn openai_responses_any_maps_to_required() {
		assert_eq!(to_openai_responses(Some(&ToolChoice::Any)), Some(serde_json::json!("required")));
	}

	#[test]
	fn openai_responses_specific() {
		let tc = ToolChoice::Specific { name: "bash".into() };
		let expected = serde_json::json!({"type": "function", "name": "bash"});
		assert_eq!(to_openai_responses(Some(&tc)), Some(expected));
	}

	#[test]
	fn openai_responses_none_option() {
		assert_eq!(to_openai_responses(None), None);
	}
}
