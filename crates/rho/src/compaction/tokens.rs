//! Token estimation using chars/4 heuristic.
//!
//! oh-my-pi ref: `compaction.ts` `estimateTokens()` lines 195-253

use crate::{
	ai::types::{AssistantMessage, ContentBlock, Message},
	session::types::SessionEntry,
};

/// Estimate token count for a message using chars/4 heuristic.
pub fn estimate_tokens(message: &Message) -> u32 {
	let chars: usize = match message {
		Message::User(u) => u.content.len(),
		Message::Assistant(a) => estimate_assistant_chars(a),
		Message::ToolResult(t) => t.content.len(),
		Message::BashExecution(b) => b.command.len() + b.output.len(),
	};
	chars_to_tokens(chars)
}

fn estimate_assistant_chars(msg: &AssistantMessage) -> usize {
	msg.content
		.iter()
		.map(|block| match block {
			ContentBlock::Text { text } => text.len(),
			ContentBlock::Thinking { thinking } => thinking.len(),
			ContentBlock::ToolUse { name, input, .. } => {
				name.len() + serde_json::to_string(input).map_or(0, |s| s.len())
			},
		})
		.sum()
}

fn chars_to_tokens(chars: usize) -> u32 {
	#[allow(
		clippy::cast_possible_truncation,
		clippy::cast_sign_loss,
		reason = "character count / 4 will always fit in u32 for any practical message"
	)]
	let tokens = (chars as f64 / 4.0).ceil() as u32;
	tokens
}

/// Estimate tokens for a session entry.
pub fn estimate_entry_tokens(entry: &SessionEntry) -> u32 {
	match entry {
		SessionEntry::Message(m) => estimate_tokens(&m.message),
		SessionEntry::BranchSummary(b) => chars_to_tokens(b.summary.len()),
		SessionEntry::CustomMessage(c) => chars_to_tokens(c.content.len()),
		SessionEntry::Compaction(c) => chars_to_tokens(c.summary.len()),
		SessionEntry::ThinkingLevelChange(_)
		| SessionEntry::ModelChange(_)
		| SessionEntry::Custom(_)
		| SessionEntry::Label(_)
		| SessionEntry::TtsrInjection(_)
		| SessionEntry::SessionInit(_)
		| SessionEntry::ModeChange(_) => 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ai::types::UserMessage;

	#[test]
	fn user_message_tokens() {
		let msg = Message::User(UserMessage { content: "Hello, world!".to_owned() });
		// 13 chars / 4 = 3.25 → ceil = 4
		assert_eq!(estimate_tokens(&msg), 4);
	}

	#[test]
	fn empty_message_tokens() {
		let msg = Message::User(UserMessage { content: String::new() });
		assert_eq!(estimate_tokens(&msg), 0);
	}

	#[test]
	fn assistant_text_tokens() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::Text { text: "x".repeat(100) }],
			stop_reason: None,
			usage:       None,
		});
		// 100 chars / 4 = 25
		assert_eq!(estimate_tokens(&msg), 25);
	}

	#[test]
	fn assistant_mixed_content() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Hello".to_owned() }, // 5 chars
				ContentBlock::Thinking { thinking: "hmm".to_owned() }, // 3 chars
				ContentBlock::ToolUse {
					id:    "t1".to_owned(),
					name:  "bash".to_owned(),                // 4 chars
					input: serde_json::json!({"cmd": "ls"}), // ~12 chars as JSON
				},
			],
			stop_reason: None,
			usage:       None,
		});
		let tokens = estimate_tokens(&msg);
		assert!(tokens > 0);
		// 5 + 3 + 4 + ~12 = ~24 chars → ~6 tokens
		assert!(tokens >= 5, "Expected at least 5 tokens, got {tokens}");
	}

	#[test]
	fn tool_result_tokens() {
		let msg = Message::ToolResult(crate::ai::types::ToolResultMessage {
			tool_use_id: "t1".to_owned(),
			content:     std::sync::Arc::new("file contents here".to_owned()),
			is_error:    false,
		});
		// 18 chars / 4 = 4.5 → ceil = 5
		assert_eq!(estimate_tokens(&msg), 5);
	}

	#[test]
	fn entry_tokens_for_message() {
		let entry = SessionEntry::Message(crate::session::types::SessionMessageEntry {
			id:        "a1".to_owned(),
			parent_id: None,
			timestamp: "2026-01-01T00:00:00Z".to_owned(),
			message:   Message::User(UserMessage { content: "Hello".to_owned() }),
		});
		assert_eq!(estimate_entry_tokens(&entry), 2); // 5 chars / 4 = 1.25 → 2
	}

	#[test]
	fn entry_tokens_for_metadata() {
		let entry =
			SessionEntry::ThinkingLevelChange(crate::session::types::ThinkingLevelChangeEntry {
				id:             "t1".to_owned(),
				parent_id:      None,
				timestamp:      "2026-01-01T00:00:00Z".to_owned(),
				thinking_level: "high".to_owned(),
			});
		assert_eq!(estimate_entry_tokens(&entry), 0);
	}
}
