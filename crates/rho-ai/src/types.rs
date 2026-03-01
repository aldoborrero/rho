use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// StopReason
// ---------------------------------------------------------------------------

/// Why the assistant stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
	Stop,
	Length,
	ToolUse,
	Error,
	Aborted,
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Token usage statistics for a single request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
	pub input_tokens:       u32,
	pub output_tokens:      u32,
	pub cache_read_tokens:  u32,
	pub cache_write_tokens: u32,
}

// ---------------------------------------------------------------------------
// ContentBlock  (assistant content)
// ---------------------------------------------------------------------------

/// A block of content in an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
	Text { text: String },
	Thinking { thinking: String },
	ToolUse { id: String, name: String, input: serde_json::Value },
}

// ---------------------------------------------------------------------------
// UserContent
// ---------------------------------------------------------------------------

/// A block of content in a user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserContent {
	Text { text: String },
	Image { data: String, mime_type: String },
}

// ---------------------------------------------------------------------------
// ToolResultContent
// ---------------------------------------------------------------------------

/// A block of content in a tool result message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolResultContent {
	Text { text: Arc<String> },
	Image { data: String, mime_type: String },
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// A user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
	pub content: Vec<UserContent>,
}

/// An assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
	pub content:     Vec<ContentBlock>,
	pub stop_reason: Option<StopReason>,
	pub usage:       Option<Usage>,
}

/// A tool result message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
	pub tool_use_id: String,
	pub content:     Vec<ToolResultContent>,
	pub is_error:    bool,
}

/// A conversation message — discriminated by `role`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum Message {
	User(UserMessage),
	Assistant(AssistantMessage),
	ToolResult(ToolResultMessage),
}

// ---------------------------------------------------------------------------
// ToolDefinition
// ---------------------------------------------------------------------------

/// Definition of a tool the model can invoke.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
	pub name:         String,
	pub description:  String,
	pub input_schema: serde_json::Value,
}

// ---------------------------------------------------------------------------
// ToolChoice
// ---------------------------------------------------------------------------

/// How the model should choose tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolChoice {
	Auto,
	None,
	Any,
	Required,
	Specific { name: String },
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Everything needed to call the LLM: system prompt, messages, and tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
	pub system_prompt: Option<Arc<String>>,
	pub messages:      Vec<Message>,
	pub tools:         Vec<ToolDefinition>,
}

// ---------------------------------------------------------------------------
// CacheRetention
// ---------------------------------------------------------------------------

/// Prompt cache retention preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheRetention {
	None,
	Short,
	Long,
}

impl Default for CacheRetention {
	fn default() -> Self {
		Self::Short
	}
}

// ---------------------------------------------------------------------------
// ReasoningLevel
// ---------------------------------------------------------------------------

/// How much reasoning/thinking the model should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningLevel {
	Minimal,
	Low,
	Medium,
	High,
	XHigh,
}

// ---------------------------------------------------------------------------
// ThinkingBudgets
// ---------------------------------------------------------------------------

/// Token budgets for each thinking level (token-based providers only).
#[derive(Debug, Clone, Default)]
pub struct ThinkingBudgets {
	pub minimal: Option<u32>,
	pub low:     Option<u32>,
	pub medium:  Option<u32>,
	pub high:    Option<u32>,
	pub xhigh:   Option<u32>,
}

// ---------------------------------------------------------------------------
// RetryConfig  (defined in retry.rs, re-exported here for convenience)
// ---------------------------------------------------------------------------

pub use crate::retry::RetryConfig;

// ---------------------------------------------------------------------------
// StreamOptions
// ---------------------------------------------------------------------------

/// Options controlling a single LLM streaming request.
#[derive(Debug, Clone)]
pub struct StreamOptions {
	pub api_key:          Option<String>,
	pub temperature:      Option<f32>,
	pub max_tokens:       Option<u32>,
	pub tool_choice:      Option<ToolChoice>,
	pub cache_retention:  CacheRetention,
	pub reasoning:        Option<ReasoningLevel>,
	pub thinking_budgets: Option<ThinkingBudgets>,
	pub headers:          HashMap<String, String>,
	pub abort:            Option<tokio_util::sync::CancellationToken>,
	pub retry:            RetryConfig,
}

impl Default for StreamOptions {
	fn default() -> Self {
		Self {
			api_key:          None,
			temperature:      None,
			max_tokens:       None,
			tool_choice:      None,
			cache_retention:  CacheRetention::default(),
			reasoning:        None,
			thinking_budgets: None,
			headers:          HashMap::new(),
			abort:            None,
			retry:            RetryConfig::default(),
		}
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn user_message_text_only() {
		let msg =
			Message::User(UserMessage { content: vec![UserContent::Text { text: "hello".into() }] });
		let json = serde_json::to_value(&msg).unwrap();
		let round: Message = serde_json::from_value(json).unwrap();
		assert!(matches!(round, Message::User(_)));
	}

	#[test]
	fn assistant_message_with_tool_use() {
		let msg = AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me help.".into() },
				ContentBlock::ToolUse {
					id:    "tc_1".into(),
					name:  "read_file".into(),
					input: serde_json::json!({"path": "/tmp/test"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       Some(Usage {
				input_tokens:       100,
				output_tokens:      50,
				cache_read_tokens:  10,
				cache_write_tokens: 5,
			}),
		};
		let json = serde_json::to_value(&msg).unwrap();
		let round: AssistantMessage = serde_json::from_value(json).unwrap();
		assert_eq!(round.content.len(), 2);
		assert!(matches!(round.stop_reason, Some(StopReason::ToolUse)));
	}

	#[test]
	fn tool_result_with_error() {
		let msg = ToolResultMessage {
			tool_use_id: "tc_1".into(),
			content:     vec![ToolResultContent::Text { text: Arc::new("file not found".into()) }],
			is_error:    true,
		};
		let json = serde_json::to_value(&msg).unwrap();
		let round: ToolResultMessage = serde_json::from_value(json).unwrap();
		assert!(round.is_error);
	}

	#[test]
	fn stop_reason_serialization() {
		assert_eq!(serde_json::to_string(&StopReason::Stop).unwrap(), "\"stop\"");
		assert_eq!(serde_json::to_string(&StopReason::Length).unwrap(), "\"length\"");
		assert_eq!(serde_json::to_string(&StopReason::ToolUse).unwrap(), "\"toolUse\"");
		assert_eq!(serde_json::to_string(&StopReason::Error).unwrap(), "\"error\"");
		assert_eq!(serde_json::to_string(&StopReason::Aborted).unwrap(), "\"aborted\"");
	}

	#[test]
	fn usage_default_zeros() {
		let u = Usage::default();
		assert_eq!(u.input_tokens, 0);
		assert_eq!(u.output_tokens, 0);
	}

	#[test]
	fn context_with_tools() {
		let ctx = Context {
			system_prompt: Some(Arc::new("You are helpful.".into())),
			messages:      vec![],
			tools:         vec![ToolDefinition {
				name:         "bash".into(),
				description:  "Run a command".into(),
				input_schema: serde_json::json!({"type": "object"}),
			}],
		};
		assert_eq!(ctx.tools.len(), 1);
	}

	#[test]
	fn tool_choice_variants() {
		let auto = ToolChoice::Auto;
		let specific = ToolChoice::Specific { name: "bash".into() };
		assert!(matches!(auto, ToolChoice::Auto));
		assert!(matches!(specific, ToolChoice::Specific { .. }));
	}

	#[test]
	fn thinking_content_block() {
		let block = ContentBlock::Thinking { thinking: "Let me think...".into() };
		let json = serde_json::to_value(&block).unwrap();
		let round: ContentBlock = serde_json::from_value(json).unwrap();
		assert!(matches!(round, ContentBlock::Thinking { .. }));
	}
}
