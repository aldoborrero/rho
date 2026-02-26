//! Session context building.
//!
//! Reconstructs a [`SessionContext`] from a branch path (leaf-to-root list
//! of entries). This implements the same algorithm as `buildSessionContext()`
//! in the TypeScript reference.

use std::collections::HashMap;

use super::types::{SessionContext, SessionEntry};
use crate::ai::types::{Message, UserMessage};

/// Build a [`SessionContext`] from a branch path (leaf-to-root order).
///
/// The caller is expected to pass the result of
/// [`SessionManager::get_branch`], which returns entries from leaf to root.
///
/// # Algorithm
///
/// 1. Reverse the path to root-to-leaf order.
/// 2. Find the latest compaction entry.
/// 3. If a compaction exists, emit its summary as a user message and skip
///    entries before `first_kept_entry_id`.
/// 4. Walk the remaining entries, accumulating messages and metadata changes.
pub fn build_context(branch: &[&SessionEntry]) -> SessionContext {
	// Reverse to root-to-leaf order.
	let path: Vec<&SessionEntry> = branch.iter().copied().rev().collect();

	let mut messages: Vec<Message> = Vec::new();
	let mut thinking_level = String::new();
	let mut models: HashMap<String, String> = HashMap::new();
	let mut injected_ttsr_rules: Vec<String> = Vec::new();
	let mut mode = String::new();
	let mut mode_data: Option<serde_json::Map<String, serde_json::Value>> = None;

	// Find the latest compaction entry (last one in root-to-leaf order).
	let latest_compaction = path.iter().enumerate().rev().find_map(|(i, entry)| {
		if let SessionEntry::Compaction(c) = entry {
			Some((i, c))
		} else {
			None
		}
	});

	// Determine which entries to process.
	let entries_to_process: &[&SessionEntry] =
		if let Some((compaction_idx, compaction)) = latest_compaction {
			// Emit the compaction summary as a user message.
			messages.push(Message::User(UserMessage { content: compaction.summary.clone() }));

			// Find the index of first_kept_entry_id in the path.
			let first_kept_idx = path
				.iter()
				.position(|e| e.id() == compaction.first_kept_entry_id);

			match first_kept_idx {
				Some(idx) => &path[idx..],
				// If first_kept_entry_id not found, process entries after compaction.
				None => &path[compaction_idx + 1..],
			}
		} else {
			&path
		};

	// Walk entries and accumulate context.
	for entry in entries_to_process {
		match entry {
			SessionEntry::Message(msg_entry) => {
				match &msg_entry.message {
					Message::BashExecution(bash) if bash.exclude_from_context => {
						// Excluded from LLM context (!! command)
					},
					Message::BashExecution(bash) => {
						// Convert to user message for LLM context
						let text = format!(
							"$ {}\n{}{}",
							bash.command,
							bash.output,
							if bash.exit_code.is_none_or(|c| c != 0) {
								format!(
									"\n[exit code: {}]",
									bash
										.exit_code
										.map_or("unknown".to_owned(), |c| c.to_string())
								)
							} else {
								String::new()
							}
						);
						messages.push(Message::User(UserMessage { content: text }));
					},
					_ => {
						messages.push(msg_entry.message.clone());
					},
				}
			},
			SessionEntry::ThinkingLevelChange(tlc) => {
				thinking_level.clone_from(&tlc.thinking_level);
			},
			SessionEntry::ModelChange(mc) => {
				let role = mc.role.as_deref().unwrap_or("default");
				models.insert(role.to_owned(), mc.model.clone());
			},
			SessionEntry::BranchSummary(bs) => {
				messages.push(Message::User(UserMessage { content: bs.summary.clone() }));
			},
			SessionEntry::CustomMessage(cm) => {
				messages.push(Message::User(UserMessage { content: cm.content.clone() }));
			},
			SessionEntry::TtsrInjection(ti) => {
				injected_ttsr_rules.clone_from(&ti.injected_rules);
			},
			SessionEntry::ModeChange(mc) => {
				mode.clone_from(&mc.mode);
				mode_data.clone_from(&mc.data);
			},
			// Compaction, Custom, Label, SessionInit → skip.
			SessionEntry::Compaction(_)
			| SessionEntry::Custom(_)
			| SessionEntry::Label(_)
			| SessionEntry::SessionInit(_) => {},
		}
	}

	SessionContext { messages, thinking_level, models, injected_ttsr_rules, mode, mode_data }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::session::types::{
		BranchSummaryEntry, CompactionEntry, CustomEntry, ModeChangeEntry, ModelChangeEntry,
		SessionMessageEntry, ThinkingLevelChangeEntry, TtsrInjectionEntry,
	};

	fn ts() -> String {
		"2025-01-15T10:30:00Z".to_owned()
	}

	fn user_msg(content: &str) -> Message {
		Message::User(UserMessage { content: content.to_owned() })
	}

	fn msg_entry(id: &str, parent_id: Option<&str>, content: &str) -> SessionEntry {
		SessionEntry::Message(SessionMessageEntry {
			id:        id.to_owned(),
			parent_id: parent_id.map(|s| s.to_owned()),
			timestamp: ts(),
			message:   user_msg(content),
		})
	}

	#[test]
	fn test_build_context_simple() {
		// Two messages, returned in order.
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = msg_entry("a2", Some("a1"), "World");

		// Branch is leaf-to-root: [a2, a1].
		let branch: Vec<&SessionEntry> = vec![&e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.messages.len(), 2);
		match &ctx.messages[0] {
			Message::User(u) => assert_eq!(u.content, "Hello"),
			_ => panic!("Expected User message"),
		}
		match &ctx.messages[1] {
			Message::User(u) => assert_eq!(u.content, "World"),
			_ => panic!("Expected User message"),
		}
		assert!(ctx.thinking_level.is_empty());
		assert!(ctx.models.is_empty());
		assert!(ctx.injected_ttsr_rules.is_empty());
		assert!(ctx.mode.is_empty());
		assert!(ctx.mode_data.is_none());
	}

	#[test]
	fn test_build_context_with_compaction() {
		// Entries: msg1, msg2, compaction(summary="Summary", first_kept="a3"), msg3.
		let e1 = msg_entry("a1", None, "First");
		let e2 = msg_entry("a2", Some("a1"), "Second");
		let e3 = SessionEntry::Compaction(CompactionEntry {
			id:                  "c1".to_owned(),
			parent_id:           Some("a2".to_owned()),
			timestamp:           ts(),
			summary:             "Summary of earlier conversation".to_owned(),
			short_summary:       None,
			first_kept_entry_id: "a4".to_owned(),
			tokens_before:       10_000,
			details:             None,
			preserve_data:       None,
			from_extension:      None,
		});
		let e4 = msg_entry("a4", Some("c1"), "Kept message");

		// Branch is leaf-to-root: [a4, c1, a2, a1].
		let branch: Vec<&SessionEntry> = vec![&e4, &e3, &e2, &e1];
		let ctx = build_context(&branch);

		// Should have: compaction summary + kept message.
		assert_eq!(ctx.messages.len(), 2);
		match &ctx.messages[0] {
			Message::User(u) => assert_eq!(u.content, "Summary of earlier conversation"),
			_ => panic!("Expected User message (compaction summary)"),
		}
		match &ctx.messages[1] {
			Message::User(u) => assert_eq!(u.content, "Kept message"),
			_ => panic!("Expected User message (kept)"),
		}
	}

	#[test]
	fn test_build_context_thinking_level() {
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
			id:             "t1".to_owned(),
			parent_id:      Some("a1".to_owned()),
			timestamp:      ts(),
			thinking_level: "high".to_owned(),
		});
		let e3 = msg_entry("a3", Some("t1"), "After thinking change");

		// Branch is leaf-to-root: [a3, t1, a1].
		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.thinking_level, "high");
		assert_eq!(ctx.messages.len(), 2);
	}

	#[test]
	fn test_build_context_model_change() {
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::ModelChange(ModelChangeEntry {
			id:        "m1".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			model:     "anthropic/claude-sonnet-4-20250514".to_owned(),
			role:      Some("default".to_owned()),
		});
		let e3 = SessionEntry::ModelChange(ModelChangeEntry {
			id:        "m2".to_owned(),
			parent_id: Some("m1".to_owned()),
			timestamp: ts(),
			model:     "anthropic/claude-haiku-3".to_owned(),
			role:      Some("smol".to_owned()),
		});
		let e4 = msg_entry("a4", Some("m2"), "After model changes");

		// Branch is leaf-to-root: [a4, m2, m1, a1].
		let branch: Vec<&SessionEntry> = vec![&e4, &e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.models.len(), 2);
		assert_eq!(ctx.models.get("default").unwrap(), "anthropic/claude-sonnet-4-20250514");
		assert_eq!(ctx.models.get("smol").unwrap(), "anthropic/claude-haiku-3");
		assert_eq!(ctx.messages.len(), 2);
	}

	#[test]
	fn test_build_context_branch_summary() {
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::BranchSummary(BranchSummaryEntry {
			id:             "b1".to_owned(),
			parent_id:      Some("a1".to_owned()),
			timestamp:      ts(),
			from_id:        "a1".to_owned(),
			summary:        "Branch summary text".to_owned(),
			details:        None,
			from_extension: None,
		});
		let e3 = msg_entry("a3", Some("b1"), "After branch");

		// Branch is leaf-to-root: [a3, b1, a1].
		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.messages.len(), 3);
		match &ctx.messages[0] {
			Message::User(u) => assert_eq!(u.content, "Hello"),
			_ => panic!("Expected User message"),
		}
		match &ctx.messages[1] {
			Message::User(u) => assert_eq!(u.content, "Branch summary text"),
			_ => panic!("Expected User message (branch summary)"),
		}
		match &ctx.messages[2] {
			Message::User(u) => assert_eq!(u.content, "After branch"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_build_context_mode_change() {
		let e1 = msg_entry("a1", None, "Hello");
		let mut data = serde_json::Map::new();
		data.insert("key".to_owned(), serde_json::json!("value"));
		let e2 = SessionEntry::ModeChange(ModeChangeEntry {
			id:        "mc1".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			mode:      "code".to_owned(),
			data:      Some(data.clone()),
		});
		let e3 = msg_entry("a3", Some("mc1"), "In code mode");

		// Branch is leaf-to-root: [a3, mc1, a1].
		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.mode, "code");
		assert!(ctx.mode_data.is_some());
		let md = ctx.mode_data.unwrap();
		assert_eq!(md.get("key").unwrap(), &serde_json::json!("value"));
		assert_eq!(ctx.messages.len(), 2);
	}

	#[test]
	fn test_build_context_empty() {
		let branch: Vec<&SessionEntry> = vec![];
		let ctx = build_context(&branch);

		assert!(ctx.messages.is_empty());
		assert!(ctx.thinking_level.is_empty());
		assert!(ctx.models.is_empty());
		assert!(ctx.injected_ttsr_rules.is_empty());
		assert!(ctx.mode.is_empty());
		assert!(ctx.mode_data.is_none());
	}

	#[test]
	fn test_build_context_skipped_entries() {
		// Custom, Label, and SessionInit entries should be skipped.
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::Custom(CustomEntry {
			id:          "x1".to_owned(),
			parent_id:   Some("a1".to_owned()),
			timestamp:   ts(),
			custom_type: "test".to_owned(),
			data:        None,
		});
		let e3 = msg_entry("a3", Some("x1"), "After custom");

		// Branch is leaf-to-root: [a3, x1, a1].
		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		// Custom entry skipped; only the two messages remain.
		assert_eq!(ctx.messages.len(), 2);
		match &ctx.messages[0] {
			Message::User(u) => assert_eq!(u.content, "Hello"),
			_ => panic!("Expected User message"),
		}
		match &ctx.messages[1] {
			Message::User(u) => assert_eq!(u.content, "After custom"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_build_context_ttsr_injection() {
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::TtsrInjection(TtsrInjectionEntry {
			id:             "ti1".to_owned(),
			parent_id:      Some("a1".to_owned()),
			timestamp:      ts(),
			injected_rules: vec!["rule1".to_owned(), "rule2".to_owned()],
		});
		let e3 = msg_entry("a3", Some("ti1"), "After injection");

		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.injected_ttsr_rules, vec!["rule1", "rule2"]);
		assert_eq!(ctx.messages.len(), 2);
	}

	#[test]
	fn test_build_context_custom_message() {
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::CustomMessage(super::super::types::CustomMessageEntry {
			id:          "cm1".to_owned(),
			parent_id:   Some("a1".to_owned()),
			timestamp:   ts(),
			custom_type: "info".to_owned(),
			content:     "Custom content here".to_owned(),
			details:     None,
			display:     true,
		});
		let e3 = msg_entry("a3", Some("cm1"), "After custom message");

		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.messages.len(), 3);
		match &ctx.messages[1] {
			Message::User(u) => assert_eq!(u.content, "Custom content here"),
			_ => panic!("Expected User message (custom message content)"),
		}
	}

	#[test]
	fn test_build_context_model_change_no_role_defaults() {
		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::ModelChange(ModelChangeEntry {
			id:        "m1".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			model:     "anthropic/claude-sonnet-4-20250514".to_owned(),
			role:      None, // No role specified — should default to "default".
		});

		let branch: Vec<&SessionEntry> = vec![&e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.models.len(), 1);
		assert_eq!(ctx.models.get("default").unwrap(), "anthropic/claude-sonnet-4-20250514");
	}

	#[test]
	fn test_build_context_bash_execution_included() {
		use crate::ai::types::BashExecutionMessage;

		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::Message(SessionMessageEntry {
			id:        "a2".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			message:   Message::BashExecution(BashExecutionMessage {
				command:              "ls".to_owned(),
				output:               "file.txt".to_owned(),
				exit_code:            Some(0),
				cancelled:            false,
				truncated:            false,
				exclude_from_context: false,
				timestamp:            1_706_000_000,
			}),
		});
		let e3 = msg_entry("a3", Some("a2"), "After bash");

		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		// Should have 3 messages: Hello, bash output (as user msg), After bash
		assert_eq!(ctx.messages.len(), 3);
		match &ctx.messages[1] {
			Message::User(u) => {
				assert!(u.content.contains("ls"), "Should contain command");
				assert!(u.content.contains("file.txt"), "Should contain output");
			},
			_ => panic!("BashExecution should be converted to User message"),
		}
	}

	#[test]
	fn test_build_context_bash_execution_excluded() {
		use crate::ai::types::BashExecutionMessage;

		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::Message(SessionMessageEntry {
			id:        "a2".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			message:   Message::BashExecution(BashExecutionMessage {
				command:              "ls".to_owned(),
				output:               "file.txt".to_owned(),
				exit_code:            Some(0),
				cancelled:            false,
				truncated:            false,
				exclude_from_context: true,
				timestamp:            1_706_000_000,
			}),
		});
		let e3 = msg_entry("a3", Some("a2"), "After bash");

		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		// Should have 2 messages: Hello, After bash (bash excluded)
		assert_eq!(ctx.messages.len(), 2);
	}

	#[test]
	fn test_build_context_latest_thinking_level_wins() {
		// Two thinking level changes; the later one should win.
		let e1 = SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
			id:             "t1".to_owned(),
			parent_id:      None,
			timestamp:      ts(),
			thinking_level: "low".to_owned(),
		});
		let e2 = SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
			id:             "t2".to_owned(),
			parent_id:      Some("t1".to_owned()),
			timestamp:      ts(),
			thinking_level: "high".to_owned(),
		});
		let e3 = msg_entry("a3", Some("t2"), "After changes");

		// Branch is leaf-to-root: [a3, t2, t1].
		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		assert_eq!(ctx.thinking_level, "high");
	}
}
