use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{models::Api, types::*};

/// Normalize a tool call ID for cross-provider compatibility.
///
/// OpenAI Responses API generates IDs that are 450+ chars with special
/// characters like `|`. Anthropic APIs require IDs matching `^[a-zA-Z0-9_-]+$`
/// (max 64 chars).
///
/// Rules:
/// 1. If `id` contains `|`, take only the part before the first `|`
/// 2. Strip characters that aren't alphanumeric, `_`, or `-` (for Anthropic)
/// 3. Truncate to 64 characters (for Anthropic)
/// 4. For OpenAI APIs, constraints are less strict but pipe-separated IDs are
///    still normalized
pub fn normalize_tool_call_id(id: &str, target_api: Api) -> String {
	// Handle pipe-separated IDs (e.g. from OpenAI Responses API:
	// "call_abc|item_xyz")
	let base = if id.contains('|') {
		id.split('|').next().unwrap_or(id)
	} else {
		id
	};

	match target_api {
		Api::AnthropicMessages => {
			// Anthropic: only alphanumeric, underscore, hyphen; max 64 chars
			let sanitized: String = base
				.chars()
				.filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
				.collect();
			if sanitized.len() > 64 {
				sanitized[..64].to_string()
			} else {
				sanitized
			}
		},
		Api::OpenAICompletions | Api::OpenAIResponses => {
			// OpenAI: less strict, but still normalize pipe-separated IDs
			base.to_string()
		},
	}
}

/// Create a synthetic error `ToolResult` message for an orphaned tool call.
fn synthetic_tool_result(tool_use_id: String) -> Message {
	Message::ToolResult(ToolResultMessage {
		tool_use_id,
		content: vec![ToolResultContent::Text {
			text: Arc::new("Tool execution was interrupted".into()),
		}],
		is_error: true,
	})
}

/// Transform messages for cross-provider compatibility.
///
/// Key operations:
/// 1. Iterate messages, track tool call IDs from assistant messages with
///    ToolUse content blocks
/// 2. Track which tool call IDs have been "answered" by ToolResult messages
/// 3. Before each user message, insert synthetic error ToolResult messages for
///    orphaned tool calls
/// 4. At end of messages, insert synthetic results for any remaining orphaned
///    tool calls
/// 5. Normalize tool call IDs based on target API constraints
/// 6. Clone messages (since we return `Vec<Message>`)
pub fn transform_messages(messages: &[Message], target_api: Api) -> Vec<Message> {
	if messages.is_empty() {
		return Vec::new();
	}

	// Build a map of original tool call IDs -> normalized IDs for the first pass
	let mut id_map: HashMap<String, String> = HashMap::new();

	// First pass: normalize tool call IDs and collect the ID mapping
	let normalized: Vec<Message> = messages
		.iter()
		.map(|msg| match msg {
			Message::Assistant(a) => {
				let new_content: Vec<ContentBlock> = a
					.content
					.iter()
					.map(|block| match block {
						ContentBlock::ToolUse { id, name, input } => {
							let normalized_id = normalize_tool_call_id(id, target_api);
							if normalized_id != *id {
								id_map.insert(id.clone(), normalized_id.clone());
							}
							ContentBlock::ToolUse {
								id:    normalized_id,
								name:  name.clone(),
								input: input.clone(),
							}
						},
						other => other.clone(),
					})
					.collect();
				Message::Assistant(AssistantMessage {
					content:     new_content,
					stop_reason: a.stop_reason.clone(),
					usage:       a.usage.clone(),
				})
			},
			Message::ToolResult(tr) => {
				let normalized_id = id_map
					.get(&tr.tool_use_id)
					.cloned()
					.unwrap_or_else(|| normalize_tool_call_id(&tr.tool_use_id, target_api));
				Message::ToolResult(ToolResultMessage {
					tool_use_id: normalized_id,
					content:     tr.content.clone(),
					is_error:    tr.is_error,
				})
			},
			Message::User(u) => Message::User(u.clone()),
		})
		.collect();

	// Second pass: insert synthetic tool results for orphaned tool calls
	let mut result: Vec<Message> = Vec::new();
	// Pending tool call IDs from the most recent assistant message (in order)
	let mut pending_tool_call_ids: Vec<String> = Vec::new();
	// Tool call IDs that have been answered by a ToolResult
	let mut answered_ids: HashSet<String> = HashSet::new();

	for msg in &normalized {
		match msg {
			Message::Assistant(a) => {
				// If we have orphaned tool calls from a *previous* assistant message,
				// flush synthetic results before this new assistant message.
				flush_orphaned(&mut result, &pending_tool_call_ids, &answered_ids);

				// Reset tracking for this assistant message
				pending_tool_call_ids = Vec::new();
				answered_ids = HashSet::new();

				// Collect tool call IDs from this assistant message
				for block in &a.content {
					if let ContentBlock::ToolUse { id, .. } = block {
						pending_tool_call_ids.push(id.clone());
					}
				}

				result.push(msg.clone());
			},
			Message::ToolResult(tr) => {
				answered_ids.insert(tr.tool_use_id.clone());
				result.push(msg.clone());
			},
			Message::User(_) => {
				// User message interrupts tool flow - insert synthetic results for orphaned
				// calls
				flush_orphaned(&mut result, &pending_tool_call_ids, &answered_ids);
				pending_tool_call_ids = Vec::new();
				answered_ids = HashSet::new();

				result.push(msg.clone());
			},
		}
	}

	// Handle orphaned tool calls at the end of the message array
	flush_orphaned(&mut result, &pending_tool_call_ids, &answered_ids);

	result
}

/// Insert synthetic error ToolResult messages for any pending tool call IDs
/// that have not been answered.
fn flush_orphaned(
	result: &mut Vec<Message>,
	pending_ids: &[String],
	answered_ids: &HashSet<String>,
) {
	for id in pending_ids {
		if !answered_ids.contains(id) {
			result.push(synthetic_tool_result(id.clone()));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn passthrough_same_provider() {
		let messages = vec![Message::User(UserMessage {
			content: vec![UserContent::Text { text: "hi".into() }],
		})];
		let result = transform_messages(&messages, Api::AnthropicMessages);
		assert_eq!(result.len(), 1);
	}

	#[test]
	fn normalize_tool_id_strips_invalid_chars() {
		assert_eq!(normalize_tool_call_id("abc!@#def", Api::AnthropicMessages), "abcdef");
	}

	#[test]
	fn normalize_tool_id_truncates_to_64() {
		let long_id = "a".repeat(100);
		let result = normalize_tool_call_id(&long_id, Api::AnthropicMessages);
		assert!(result.len() <= 64);
	}

	#[test]
	fn normalize_tool_id_pipe_separated() {
		let id = "call_abc|item_xyz";
		let result = normalize_tool_call_id(id, Api::AnthropicMessages);
		assert_eq!(result, "call_abc");
	}

	#[test]
	fn orphaned_tool_call_gets_synthetic_result() {
		let messages = vec![
			Message::Assistant(AssistantMessage {
				content:     vec![ContentBlock::ToolUse {
					id:    "tc_1".into(),
					name:  "bash".into(),
					input: serde_json::json!({}),
				}],
				stop_reason: Some(StopReason::ToolUse),
				usage:       None,
			}),
			// No ToolResult for tc_1
			Message::User(UserMessage {
				content: vec![UserContent::Text { text: "continue".into() }],
			}),
		];
		let result = transform_messages(&messages, Api::AnthropicMessages);
		// Should have: Assistant, synthetic ToolResult, User
		assert_eq!(result.len(), 3);
		assert!(matches!(&result[1], Message::ToolResult(tr) if tr.is_error));
	}

	#[test]
	fn empty_messages_returns_empty() {
		let result = transform_messages(&[], Api::AnthropicMessages);
		assert!(result.is_empty());
	}

	#[test]
	fn tool_result_is_not_orphaned() {
		// When a tool result is provided for a tool call, no synthetic result should be
		// inserted
		let messages = vec![
			Message::Assistant(AssistantMessage {
				content:     vec![ContentBlock::ToolUse {
					id:    "tc_1".into(),
					name:  "bash".into(),
					input: serde_json::json!({}),
				}],
				stop_reason: Some(StopReason::ToolUse),
				usage:       None,
			}),
			Message::ToolResult(ToolResultMessage {
				tool_use_id: "tc_1".into(),
				content:     vec![ToolResultContent::Text { text: Arc::new("done".into()) }],
				is_error:    false,
			}),
			Message::User(UserMessage { content: vec![UserContent::Text { text: "thanks".into() }] }),
		];
		let result = transform_messages(&messages, Api::AnthropicMessages);
		// Should remain: Assistant, ToolResult, User (no synthetic)
		assert_eq!(result.len(), 3);
		assert!(matches!(&result[1], Message::ToolResult(tr) if !tr.is_error));
	}

	#[test]
	fn orphaned_tool_call_at_end_gets_synthetic_result() {
		// When the last message is an assistant with a tool call and no result follows
		let messages = vec![Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "tc_end".into(),
				name:  "bash".into(),
				input: serde_json::json!({}),
			}],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		})];
		let result = transform_messages(&messages, Api::AnthropicMessages);
		// Should have: Assistant, synthetic ToolResult
		assert_eq!(result.len(), 2);
		assert!(
			matches!(&result[1], Message::ToolResult(tr) if tr.is_error && tr.tool_use_id == "tc_end")
		);
	}

	#[test]
	fn multiple_orphaned_tool_calls() {
		let messages = vec![
			Message::Assistant(AssistantMessage {
				content:     vec![
					ContentBlock::ToolUse {
						id:    "tc_a".into(),
						name:  "bash".into(),
						input: serde_json::json!({}),
					},
					ContentBlock::ToolUse {
						id:    "tc_b".into(),
						name:  "read".into(),
						input: serde_json::json!({}),
					},
				],
				stop_reason: Some(StopReason::ToolUse),
				usage:       None,
			}),
			// Only tc_a gets a result
			Message::ToolResult(ToolResultMessage {
				tool_use_id: "tc_a".into(),
				content:     vec![ToolResultContent::Text { text: Arc::new("ok".into()) }],
				is_error:    false,
			}),
			Message::User(UserMessage { content: vec![UserContent::Text { text: "next".into() }] }),
		];
		let result = transform_messages(&messages, Api::AnthropicMessages);
		// Should have: Assistant, ToolResult(tc_a), synthetic ToolResult(tc_b), User
		assert_eq!(result.len(), 4);
		assert!(
			matches!(&result[2], Message::ToolResult(tr) if tr.is_error && tr.tool_use_id == "tc_b")
		);
	}

	#[test]
	fn normalize_tool_id_openai_pipe_separated() {
		let id = "call_abc|item_xyz";
		let result = normalize_tool_call_id(id, Api::OpenAICompletions);
		assert_eq!(result, "call_abc");
	}

	#[test]
	fn normalize_tool_id_openai_no_pipe() {
		let id = "call_abc123";
		let result = normalize_tool_call_id(id, Api::OpenAICompletions);
		assert_eq!(result, "call_abc123");
	}

	#[test]
	fn normalize_tool_id_preserves_valid_chars() {
		let id = "toolu_abc-123_XYZ";
		let result = normalize_tool_call_id(id, Api::AnthropicMessages);
		assert_eq!(result, "toolu_abc-123_XYZ");
	}

	#[test]
	fn tool_call_ids_are_normalized_in_output() {
		let messages = vec![
			Message::Assistant(AssistantMessage {
				content:     vec![ContentBlock::ToolUse {
					id:    "tc!@#1".into(),
					name:  "bash".into(),
					input: serde_json::json!({}),
				}],
				stop_reason: Some(StopReason::ToolUse),
				usage:       None,
			}),
			Message::ToolResult(ToolResultMessage {
				tool_use_id: "tc!@#1".into(),
				content:     vec![ToolResultContent::Text { text: Arc::new("ok".into()) }],
				is_error:    false,
			}),
		];
		let result = transform_messages(&messages, Api::AnthropicMessages);
		assert_eq!(result.len(), 2);
		// Both the ToolUse ID and the ToolResult tool_use_id should be normalized
		if let Message::Assistant(a) = &result[0] {
			if let ContentBlock::ToolUse { id, .. } = &a.content[0] {
				assert_eq!(id, "tc1");
			} else {
				panic!("expected ToolUse");
			}
		} else {
			panic!("expected Assistant");
		}
		if let Message::ToolResult(tr) = &result[1] {
			assert_eq!(tr.tool_use_id, "tc1");
		} else {
			panic!("expected ToolResult");
		}
	}
}
