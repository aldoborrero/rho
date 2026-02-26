//! `/copy` and `/dump` command handlers.

use std::fmt::Write as _;

use super::super::types::{CommandContext, CommandResult};
use crate::ai::types::{AssistantMessage, ContentBlock, Message};

/// Handler for `/copy` — copy the last assistant message to the clipboard.
#[allow(
	clippy::future_not_send,
	reason = "CommandContext borrows SessionManager which contains dyn SessionStorage (not Sync)"
)]
pub async fn cmd_copy(ctx: &CommandContext<'_>) -> anyhow::Result<CommandResult> {
	let last_assistant = ctx.session.messages().iter().rev().find_map(|msg| {
		if let Message::Assistant(assistant) = msg {
			Some(assistant)
		} else {
			None
		}
	});

	let Some(assistant) = last_assistant else {
		return Ok(CommandResult::Message("No assistant messages to copy.".to_owned()));
	};

	let text = extract_assistant_text(assistant);
	if text.is_empty() {
		return Ok(CommandResult::Message("Last assistant message has no text content.".to_owned()));
	}

	let text_to_copy = text.clone();
	tokio::task::spawn_blocking(move || rho_tools::clipboard::copy_to_clipboard(text_to_copy))
		.await
		.map_err(|e| anyhow::anyhow!("Clipboard task failed: {e}"))??;

	Ok(CommandResult::Message("Copied to clipboard.".to_owned()))
}

/// Handler for `/dump` — copy the entire transcript to the clipboard.
#[allow(
	clippy::future_not_send,
	reason = "CommandContext borrows SessionManager which contains dyn SessionStorage (not Sync)"
)]
pub async fn cmd_dump(ctx: &CommandContext<'_>) -> anyhow::Result<CommandResult> {
	let messages = ctx.session.messages();
	if messages.is_empty() {
		return Ok(CommandResult::Message("No messages to copy.".to_owned()));
	}

	let mut transcript = String::new();
	for msg in messages {
		match msg {
			Message::User(u) => {
				let _ = writeln!(transcript, "User: {}\n", u.content);
			},
			Message::Assistant(a) => {
				let text = extract_assistant_text(a);
				if !text.is_empty() {
					let _ = writeln!(transcript, "Assistant: {text}\n");
				}
			},
			Message::ToolResult(t) => {
				let label = if t.is_error {
					"Tool Error"
				} else {
					"Tool Result"
				};
				let _ = writeln!(transcript, "{label}: {}\n", t.content);
			},
			Message::BashExecution(b) => {
				let _ = writeln!(transcript, "Bash: $ {}\n{}\n", b.command, b.output);
			},
		}
	}

	let text_to_copy = transcript;
	tokio::task::spawn_blocking(move || rho_tools::clipboard::copy_to_clipboard(text_to_copy))
		.await
		.map_err(|e| anyhow::anyhow!("Clipboard task failed: {e}"))??;

	Ok(CommandResult::Message("Transcript copied to clipboard.".to_owned()))
}

/// Extract all text content from an assistant message.
pub fn extract_assistant_text(assistant: &AssistantMessage) -> String {
	let mut parts = Vec::new();
	for block in &assistant.content {
		if let ContentBlock::Text { text } = block {
			parts.push(text.as_str());
		}
	}
	parts.join("\n")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn extract_text_from_assistant() {
		let msg = AssistantMessage {
			content:     vec![ContentBlock::Text { text: "Hello".to_owned() }, ContentBlock::Text {
				text: "World".to_owned(),
			}],
			stop_reason: None,
			usage:       None,
		};
		assert_eq!(extract_assistant_text(&msg), "Hello\nWorld");
	}

	#[test]
	fn extract_text_skips_non_text_blocks() {
		let msg = AssistantMessage {
			content:     vec![
				ContentBlock::Thinking { thinking: "thinking...".to_owned() },
				ContentBlock::Text { text: "answer".to_owned() },
			],
			stop_reason: None,
			usage:       None,
		};
		assert_eq!(extract_assistant_text(&msg), "answer");
	}

	#[test]
	fn extract_text_empty() {
		let msg = AssistantMessage { content: vec![], stop_reason: None, usage: None };
		assert_eq!(extract_assistant_text(&msg), "");
	}
}
