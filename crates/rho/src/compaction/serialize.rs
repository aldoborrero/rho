//! Conversation serializer for the summarization prompt.
//!
//! Converts `rho_agent::types::Message` values to a text representation
//! that prevents the summarization LLM from continuing the conversation.
//!
//! oh-my-pi ref: `compaction/utils.ts` `serializeConversation()`

use crate::ai::types::{AssistantMessage, ContentBlock, Message};

/// Serialize messages to a text representation for the summarization prompt.
///
/// Format:
/// ```text
/// [User]: {text}
/// [Assistant thinking]: {thinking blocks}
/// [Assistant]: {text blocks}
/// [Assistant tool calls]: name({"arg": "value"}); ...
/// [Tool result]: {content}
/// ```
pub fn serialize_conversation(messages: &[Message]) -> String {
	let mut output = String::new();
	for message in messages {
		match message {
			Message::User(u) => {
				output.push_str("[User]: ");
				output.push_str(&u.content);
				output.push('\n');
			},
			Message::Assistant(a) => {
				serialize_assistant(&mut output, a);
			},
			Message::ToolResult(t) => {
				output.push_str("[Tool result]: ");
				output.push_str(&t.content);
				output.push('\n');
			},
			Message::BashExecution(b) => {
				output.push_str("[Bash execution]: $ ");
				output.push_str(&b.command);
				output.push('\n');
				output.push_str(&b.output);
				output.push('\n');
			},
		}
	}
	output
}

fn serialize_assistant(output: &mut String, msg: &AssistantMessage) {
	let mut tool_calls = Vec::new();

	for block in &msg.content {
		match block {
			ContentBlock::Thinking { thinking } => {
				output.push_str("[Assistant thinking]: ");
				output.push_str(thinking);
				output.push('\n');
			},
			ContentBlock::Text { text } => {
				output.push_str("[Assistant]: ");
				output.push_str(text);
				output.push('\n');
			},
			ContentBlock::ToolUse { name, input, .. } => {
				let args = serde_json::to_string(input).unwrap_or_default();
				tool_calls.push(format!("{name}({args})"));
			},
		}
	}

	if !tool_calls.is_empty() {
		output.push_str("[Assistant tool calls]: ");
		output.push_str(&tool_calls.join("; "));
		output.push('\n');
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use super::*;
	use crate::ai::types::{ToolResultMessage, UserMessage};

	#[test]
	fn serialize_user_message() {
		let msgs = vec![Message::User(UserMessage { content: "Hello!".to_owned() })];
		let result = serialize_conversation(&msgs);
		assert_eq!(result, "[User]: Hello!\n");
	}

	#[test]
	fn serialize_tool_result() {
		let msgs = vec![Message::ToolResult(ToolResultMessage {
			tool_use_id: "t1".to_owned(),
			content:     Arc::new("file contents".to_owned()),
			is_error:    false,
		})];
		let result = serialize_conversation(&msgs);
		assert_eq!(result, "[Tool result]: file contents\n");
	}

	#[test]
	fn serialize_assistant_text() {
		let msgs = vec![Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::Text { text: "I can help.".to_owned() }],
			stop_reason: None,
			usage:       None,
		})];
		let result = serialize_conversation(&msgs);
		assert_eq!(result, "[Assistant]: I can help.\n");
	}

	#[test]
	fn serialize_assistant_with_tool_calls() {
		let msgs = vec![Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me check.".to_owned() },
				ContentBlock::ToolUse {
					id:    "t1".to_owned(),
					name:  "bash".to_owned(),
					input: serde_json::json!({"command": "ls"}),
				},
			],
			stop_reason: None,
			usage:       None,
		})];
		let result = serialize_conversation(&msgs);
		assert!(result.contains("[Assistant]: Let me check."));
		assert!(result.contains("[Assistant tool calls]: bash("));
	}

	#[test]
	fn serialize_assistant_thinking() {
		let msgs = vec![Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Thinking { thinking: "Let me think...".to_owned() },
				ContentBlock::Text { text: "Here's my answer.".to_owned() },
			],
			stop_reason: None,
			usage:       None,
		})];
		let result = serialize_conversation(&msgs);
		assert!(result.contains("[Assistant thinking]: Let me think..."));
		assert!(result.contains("[Assistant]: Here's my answer."));
	}

	#[test]
	fn serialize_full_conversation() {
		let msgs = vec![
			Message::User(UserMessage { content: "Fix the bug".to_owned() }),
			Message::Assistant(AssistantMessage {
				content:     vec![
					ContentBlock::Text { text: "I'll look at the code.".to_owned() },
					ContentBlock::ToolUse {
						id:    "t1".to_owned(),
						name:  "read".to_owned(),
						input: serde_json::json!({"path": "src/main.rs"}),
					},
				],
				stop_reason: None,
				usage:       None,
			}),
			Message::ToolResult(ToolResultMessage {
				tool_use_id: "t1".to_owned(),
				content:     Arc::new("fn main() {}".to_owned()),
				is_error:    false,
			}),
			Message::Assistant(AssistantMessage {
				content:     vec![ContentBlock::Text { text: "Found the issue.".to_owned() }],
				stop_reason: None,
				usage:       None,
			}),
		];
		let result = serialize_conversation(&msgs);
		assert!(result.contains("[User]: Fix the bug"));
		assert!(result.contains("[Assistant]: I'll look at the code."));
		assert!(result.contains("[Assistant tool calls]: read("));
		assert!(result.contains("[Tool result]: fn main() {}"));
		assert!(result.contains("[Assistant]: Found the issue."));
	}
}
