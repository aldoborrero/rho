use serde::{Deserialize, Serialize};

// === Internal Message Types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Message {
	#[serde(rename = "user")]
	User(UserMessage),
	#[serde(rename = "assistant")]
	Assistant(AssistantMessage),
	#[serde(rename = "tool_result")]
	ToolResult(ToolResultMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
	pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
	pub content:     Vec<ContentBlock>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_reason: Option<StopReason>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage:       Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
	pub tool_use_id: String,
	pub content:     String,
	#[serde(default)]
	pub is_error:    bool,
}

// === Content Blocks ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
	#[serde(rename = "text")]
	Text { text: String },
	#[serde(rename = "thinking")]
	Thinking { thinking: String },
	#[serde(rename = "tool_use")]
	ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_field_names, reason = "field names match the Anthropic API schema")]
pub struct Usage {
	pub input_tokens:                u32,
	pub output_tokens:               u32,
	#[serde(default)]
	pub cache_creation_input_tokens: u32,
	#[serde(default)]
	pub cache_read_input_tokens:     u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
	EndTurn,
	ToolUse,
	MaxTokens,
	StopSequence,
}

// === Tool Definition ===

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
	pub name:         String,
	pub description:  String,
	pub input_schema: serde_json::Value,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_user_message_roundtrip() {
		let msg = Message::User(UserMessage { content: "Hello".to_owned() });
		let json = serde_json::to_string(&msg).unwrap();
		let parsed: Message = serde_json::from_str(&json).unwrap();
		match parsed {
			Message::User(u) => assert_eq!(u.content, "Hello"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_assistant_message_with_tool_use() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me check.".to_owned() },
				ContentBlock::ToolUse {
					id:    "tu_1".to_owned(),
					name:  "bash".to_owned(),
					input: serde_json::json!({"command": "ls"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let json = serde_json::to_string(&msg).unwrap();
		assert!(json.contains("tool_use"));
		let parsed: Message = serde_json::from_str(&json).unwrap();
		match parsed {
			Message::Assistant(a) => assert_eq!(a.content.len(), 2),
			_ => panic!("Expected Assistant message"),
		}
	}
}
