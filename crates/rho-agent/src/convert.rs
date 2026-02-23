use crate::types::{AssistantMessage, ContentBlock, Message, StopReason, Usage};

// ---------------------------------------------------------------------------
// Agent -> rho-ai conversions
// ---------------------------------------------------------------------------

/// Convert agent messages to rho-ai messages for provider calls.
pub fn to_ai_messages(messages: &[Message]) -> Vec<rho_ai::types::Message> {
	messages.iter().map(to_ai_message).collect()
}

fn to_ai_message(msg: &Message) -> rho_ai::types::Message {
	match msg {
		Message::User(u) => rho_ai::types::Message::User(rho_ai::types::UserMessage {
			content: vec![rho_ai::types::UserContent::Text { text: u.content.clone() }],
		}),
		Message::Assistant(a) => rho_ai::types::Message::Assistant(rho_ai::types::AssistantMessage {
			content:     a.content.iter().map(to_ai_content_block).collect(),
			stop_reason: a.stop_reason.as_ref().map(to_ai_stop_reason),
			usage:       a.usage.as_ref().map(to_ai_usage),
		}),
		Message::ToolResult(t) => {
			rho_ai::types::Message::ToolResult(rho_ai::types::ToolResultMessage {
				tool_use_id: t.tool_use_id.clone(),
				content:     vec![rho_ai::types::ToolResultContent::Text { text: t.content.clone() }],
				is_error:    t.is_error,
			})
		},
	}
}

fn to_ai_content_block(block: &ContentBlock) -> rho_ai::types::ContentBlock {
	match block {
		ContentBlock::Text { text } => rho_ai::types::ContentBlock::Text { text: text.clone() },
		ContentBlock::Thinking { thinking } => {
			rho_ai::types::ContentBlock::Thinking { thinking: thinking.clone() }
		},
		ContentBlock::ToolUse { id, name, input } => rho_ai::types::ContentBlock::ToolUse {
			id:    id.clone(),
			name:  name.clone(),
			input: input.clone(),
		},
	}
}

const fn to_ai_stop_reason(reason: &StopReason) -> rho_ai::types::StopReason {
	match reason {
		StopReason::EndTurn | StopReason::StopSequence => rho_ai::types::StopReason::Stop,
		StopReason::MaxTokens => rho_ai::types::StopReason::Length,
		StopReason::ToolUse => rho_ai::types::StopReason::ToolUse,
	}
}

const fn to_ai_usage(usage: &Usage) -> rho_ai::types::Usage {
	rho_ai::types::Usage {
		input_tokens:       usage.input_tokens,
		output_tokens:      usage.output_tokens,
		cache_read_tokens:  usage.cache_read_input_tokens,
		cache_write_tokens: usage.cache_creation_input_tokens,
	}
}

// ---------------------------------------------------------------------------
// rho-ai -> Agent conversions
// ---------------------------------------------------------------------------

/// Convert a rho-ai assistant message back to an agent assistant message.
pub fn from_ai_assistant(msg: &rho_ai::types::AssistantMessage) -> AssistantMessage {
	AssistantMessage {
		content:     msg.content.iter().map(from_ai_content_block).collect(),
		stop_reason: msg.stop_reason.as_ref().map(from_ai_stop_reason),
		usage:       msg.usage.as_ref().map(from_ai_usage),
	}
}

fn from_ai_content_block(block: &rho_ai::types::ContentBlock) -> ContentBlock {
	match block {
		rho_ai::types::ContentBlock::Text { text } => ContentBlock::Text { text: text.clone() },
		rho_ai::types::ContentBlock::Thinking { thinking } => {
			ContentBlock::Thinking { thinking: thinking.clone() }
		},
		rho_ai::types::ContentBlock::ToolUse { id, name, input } => {
			ContentBlock::ToolUse { id: id.clone(), name: name.clone(), input: input.clone() }
		},
	}
}

const fn from_ai_stop_reason(reason: &rho_ai::types::StopReason) -> StopReason {
	match reason {
		rho_ai::types::StopReason::Stop => StopReason::EndTurn,
		rho_ai::types::StopReason::Length => StopReason::MaxTokens,
		rho_ai::types::StopReason::ToolUse => StopReason::ToolUse,
		rho_ai::types::StopReason::Error | rho_ai::types::StopReason::Aborted => StopReason::EndTurn,
	}
}

const fn from_ai_usage(usage: &rho_ai::types::Usage) -> Usage {
	Usage {
		input_tokens:                usage.input_tokens,
		output_tokens:               usage.output_tokens,
		cache_creation_input_tokens: usage.cache_write_tokens,
		cache_read_input_tokens:     usage.cache_read_tokens,
	}
}

// ---------------------------------------------------------------------------
// Tool definition conversion
// ---------------------------------------------------------------------------

/// Convert agent tool definitions to rho-ai format.
pub fn to_ai_tool_defs(
	defs: &[crate::types::ToolDefinition],
) -> Vec<rho_ai::types::ToolDefinition> {
	defs
		.iter()
		.map(|d| rho_ai::types::ToolDefinition {
			name:         d.name.clone(),
			description:  d.description.clone(),
			input_schema: d.input_schema.clone(),
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::{ToolResultMessage, UserMessage};

	// --- to_ai_messages ---

	#[test]
	fn test_user_message_conversion() {
		let messages = vec![Message::User(UserMessage { content: "Hello, Claude!".to_owned() })];

		let ai_messages = to_ai_messages(&messages);
		assert_eq!(ai_messages.len(), 1);
		match &ai_messages[0] {
			rho_ai::types::Message::User(u) => {
				assert_eq!(u.content.len(), 1);
				match &u.content[0] {
					rho_ai::types::UserContent::Text { text } => {
						assert_eq!(text, "Hello, Claude!");
					},
					_ => panic!("Expected Text content"),
				}
			},
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_assistant_message_with_tool_calls() {
		let messages = vec![Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me check that.".to_owned() },
				ContentBlock::ToolUse {
					id:    "tu_123".to_owned(),
					name:  "bash".to_owned(),
					input: serde_json::json!({"command": "ls -la"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		})];

		let ai_messages = to_ai_messages(&messages);
		assert_eq!(ai_messages.len(), 1);
		match &ai_messages[0] {
			rho_ai::types::Message::Assistant(a) => {
				assert_eq!(a.content.len(), 2);
				match &a.content[0] {
					rho_ai::types::ContentBlock::Text { text } => {
						assert_eq!(text, "Let me check that.");
					},
					_ => panic!("Expected Text block"),
				}
				match &a.content[1] {
					rho_ai::types::ContentBlock::ToolUse { id, name, input } => {
						assert_eq!(id, "tu_123");
						assert_eq!(name, "bash");
						assert_eq!(input, &serde_json::json!({"command": "ls -la"}));
					},
					_ => panic!("Expected ToolUse block"),
				}
			},
			_ => panic!("Expected Assistant message"),
		}
	}

	#[test]
	fn test_tool_result_conversion() {
		let messages = vec![Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_123".to_owned(),
			content:     "file1.txt\nfile2.txt".to_owned(),
			is_error:    false,
		})];

		let ai_messages = to_ai_messages(&messages);
		assert_eq!(ai_messages.len(), 1);
		match &ai_messages[0] {
			rho_ai::types::Message::ToolResult(t) => {
				assert_eq!(t.tool_use_id, "tu_123");
				assert!(!t.is_error);
				assert_eq!(t.content.len(), 1);
				match &t.content[0] {
					rho_ai::types::ToolResultContent::Text { text } => {
						assert_eq!(text, "file1.txt\nfile2.txt");
					},
					_ => panic!("Expected Text content"),
				}
			},
			_ => panic!("Expected ToolResult message"),
		}
	}

	#[test]
	fn test_tool_result_with_error() {
		let messages = vec![Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_456".to_owned(),
			content:     "command not found".to_owned(),
			is_error:    true,
		})];

		let ai_messages = to_ai_messages(&messages);
		match &ai_messages[0] {
			rho_ai::types::Message::ToolResult(t) => {
				assert!(t.is_error);
			},
			_ => panic!("Expected ToolResult message"),
		}
	}

	#[test]
	fn test_mixed_message_conversion() {
		let messages = vec![
			Message::User(UserMessage { content: "Run ls".to_owned() }),
			Message::Assistant(AssistantMessage {
				content:     vec![ContentBlock::ToolUse {
					id:    "tu_1".to_owned(),
					name:  "bash".to_owned(),
					input: serde_json::json!({"command": "ls"}),
				}],
				stop_reason: Some(StopReason::ToolUse),
				usage:       None,
			}),
			Message::ToolResult(ToolResultMessage {
				tool_use_id: "tu_1".to_owned(),
				content:     "file.txt".to_owned(),
				is_error:    false,
			}),
		];

		let ai_messages = to_ai_messages(&messages);
		assert_eq!(ai_messages.len(), 3);
		assert!(matches!(ai_messages[0], rho_ai::types::Message::User(_)));
		assert!(matches!(ai_messages[1], rho_ai::types::Message::Assistant(_)));
		assert!(matches!(ai_messages[2], rho_ai::types::Message::ToolResult(_)));
	}

	// --- from_ai_assistant ---

	#[test]
	fn test_from_ai_assistant_text() {
		let ai_msg = rho_ai::types::AssistantMessage {
			content:     vec![rho_ai::types::ContentBlock::Text { text: "Hello world!".to_owned() }],
			stop_reason: Some(rho_ai::types::StopReason::Stop),
			usage:       Some(rho_ai::types::Usage {
				input_tokens:       10,
				output_tokens:      5,
				cache_read_tokens:  0,
				cache_write_tokens: 0,
			}),
		};

		let agent_msg = from_ai_assistant(&ai_msg);
		assert_eq!(agent_msg.content.len(), 1);
		match &agent_msg.content[0] {
			ContentBlock::Text { text } => assert_eq!(text, "Hello world!"),
			_ => panic!("Expected Text block"),
		}
		assert!(matches!(agent_msg.stop_reason, Some(StopReason::EndTurn)));
		let usage = agent_msg.usage.unwrap();
		assert_eq!(usage.input_tokens, 10);
		assert_eq!(usage.output_tokens, 5);
	}

	#[test]
	fn test_from_ai_assistant_tool_use() {
		let ai_msg = rho_ai::types::AssistantMessage {
			content:     vec![
				rho_ai::types::ContentBlock::Text { text: "Let me run that.".to_owned() },
				rho_ai::types::ContentBlock::ToolUse {
					id:    "tu_abc".to_owned(),
					name:  "bash".to_owned(),
					input: serde_json::json!({"command": "ls"}),
				},
			],
			stop_reason: Some(rho_ai::types::StopReason::ToolUse),
			usage:       None,
		};

		let agent_msg = from_ai_assistant(&ai_msg);
		assert_eq!(agent_msg.content.len(), 2);
		match &agent_msg.content[1] {
			ContentBlock::ToolUse { id, name, input } => {
				assert_eq!(id, "tu_abc");
				assert_eq!(name, "bash");
				assert_eq!(input, &serde_json::json!({"command": "ls"}));
			},
			_ => panic!("Expected ToolUse block"),
		}
		assert!(matches!(agent_msg.stop_reason, Some(StopReason::ToolUse)));
	}

	#[test]
	fn test_from_ai_assistant_thinking() {
		let ai_msg = rho_ai::types::AssistantMessage {
			content:     vec![
				rho_ai::types::ContentBlock::Thinking { thinking: "Let me think...".to_owned() },
				rho_ai::types::ContentBlock::Text { text: "Here is my answer.".to_owned() },
			],
			stop_reason: Some(rho_ai::types::StopReason::Stop),
			usage:       None,
		};

		let agent_msg = from_ai_assistant(&ai_msg);
		assert_eq!(agent_msg.content.len(), 2);
		match &agent_msg.content[0] {
			ContentBlock::Thinking { thinking } => {
				assert_eq!(thinking, "Let me think...");
			},
			_ => panic!("Expected Thinking block"),
		}
	}

	#[test]
	fn test_stop_reason_roundtrip() {
		// EndTurn -> Stop -> EndTurn
		assert!(matches!(
			from_ai_stop_reason(&to_ai_stop_reason(&StopReason::EndTurn)),
			StopReason::EndTurn
		));
		// MaxTokens -> Length -> MaxTokens
		assert!(matches!(
			from_ai_stop_reason(&to_ai_stop_reason(&StopReason::MaxTokens)),
			StopReason::MaxTokens
		));
		// ToolUse -> ToolUse -> ToolUse
		assert!(matches!(
			from_ai_stop_reason(&to_ai_stop_reason(&StopReason::ToolUse)),
			StopReason::ToolUse
		));
		// StopSequence -> Stop -> EndTurn (lossy, by design)
		assert!(matches!(
			from_ai_stop_reason(&to_ai_stop_reason(&StopReason::StopSequence)),
			StopReason::EndTurn
		));
	}

	#[test]
	fn test_usage_roundtrip() {
		let usage = Usage {
			input_tokens:                100,
			output_tokens:               50,
			cache_creation_input_tokens: 20,
			cache_read_input_tokens:     10,
		};
		let ai_usage = to_ai_usage(&usage);
		let back = from_ai_usage(&ai_usage);
		assert_eq!(back.input_tokens, 100);
		assert_eq!(back.output_tokens, 50);
		assert_eq!(back.cache_creation_input_tokens, 20);
		assert_eq!(back.cache_read_input_tokens, 10);
	}

	// --- to_ai_tool_defs ---

	#[test]
	fn test_tool_def_conversion() {
		let defs = vec![crate::types::ToolDefinition {
			name:         "bash".to_owned(),
			description:  "Run a shell command".to_owned(),
			input_schema: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
		}];

		let ai_defs = to_ai_tool_defs(&defs);
		assert_eq!(ai_defs.len(), 1);
		assert_eq!(ai_defs[0].name, "bash");
		assert_eq!(ai_defs[0].description, "Run a shell command");
	}
}
