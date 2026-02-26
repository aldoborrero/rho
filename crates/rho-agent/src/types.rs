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
	#[serde(rename = "bashExecution")]
	BashExecution(BashExecutionMessage),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashExecutionMessage {
	pub command:              String,
	pub output:               String,
	pub exit_code:            Option<i32>,
	pub cancelled:            bool,
	pub truncated:            bool,
	#[serde(default)]
	pub exclude_from_context: bool,
	pub timestamp:            i64,
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
	fn test_bash_execution_message_roundtrip() {
		let msg = Message::BashExecution(BashExecutionMessage {
			command:              "ls -la".to_owned(),
			output:               "total 0\ndrwxr-xr-x 2 user user 40 Jan 1 00:00 .\n".to_owned(),
			exit_code:            Some(0),
			cancelled:            false,
			truncated:            false,
			exclude_from_context: false,
			timestamp:            1_706_000_000,
		});
		let json = serde_json::to_string(&msg).unwrap();
		assert!(json.contains("\"role\":\"bashExecution\""), "tag should be bashExecution");
		let parsed: Message = serde_json::from_str(&json).unwrap();
		match parsed {
			Message::BashExecution(b) => {
				assert_eq!(b.command, "ls -la");
				assert_eq!(b.exit_code, Some(0));
				assert!(!b.exclude_from_context);
			},
			_ => panic!("Expected BashExecution message"),
		}
	}

	#[test]
	fn test_bash_execution_exclude_from_context() {
		let msg = Message::BashExecution(BashExecutionMessage {
			command:              "pwd".to_owned(),
			output:               "/home/user".to_owned(),
			exit_code:            Some(0),
			cancelled:            false,
			truncated:            false,
			exclude_from_context: true,
			timestamp:            1_706_000_000,
		});
		let json = serde_json::to_string(&msg).unwrap();
		let parsed: Message = serde_json::from_str(&json).unwrap();
		match parsed {
			Message::BashExecution(b) => assert!(b.exclude_from_context),
			_ => panic!("Expected BashExecution message"),
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
