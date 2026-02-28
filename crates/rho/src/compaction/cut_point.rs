//! Cut-point algorithm: find where to split the conversation for compaction.
//!
//! oh-my-pi ref: `compaction.ts` `findCutPoint()` lines 357-419,
//! `findValidCutPoints()` lines 273-308

use super::tokens::estimate_entry_tokens;
use crate::ai::types::Message;
use crate::session::types::SessionEntry;

/// Result of finding where to split the conversation for compaction.
pub struct CutPointResult {
	/// Index of the first entry to keep (everything before is summarized).
	pub first_kept_index: usize,
}

/// Check if an entry is a valid cut point.
///
/// Valid: User messages, Assistant messages (turn boundaries),
/// `BranchSummary`, `CustomMessage`.
/// Invalid: `ToolResult` (must stay with its tool call), metadata entries.
const fn is_valid_cut_point(entry: &SessionEntry) -> bool {
	match entry {
		SessionEntry::Message(m) => {
			matches!(m.message, Message::User(_) | Message::Assistant(_))
		},
		SessionEntry::BranchSummary(_) | SessionEntry::CustomMessage(_) => true,
		_ => false,
	}
}

/// Find valid cut point indices in the entry range `[start, end)`.
fn find_valid_cut_points(entries: &[&SessionEntry], start: usize, end: usize) -> Vec<usize> {
	(start..end)
		.filter(|&i| is_valid_cut_point(entries[i]))
		.collect()
}

/// Find the cut point by walking backwards and accumulating token estimates.
///
/// Returns `None` if there's not enough content to compact (less than
/// `keep_recent_tokens` total tokens, or no valid cut points).
///
/// oh-my-pi ref: `compaction.ts` `findCutPoint()` lines 357-419
pub fn find_cut_point(
	entries: &[&SessionEntry],
	start: usize,
	end: usize,
	keep_recent_tokens: u32,
) -> Option<CutPointResult> {
	if start >= end {
		return None;
	}

	let valid_points = find_valid_cut_points(entries, start, end);
	if valid_points.is_empty() {
		return None;
	}

	let mut tokens_from_end: u32 = 0;

	// Walk backward through valid cut points.
	for (vi, &cut_idx) in valid_points.iter().enumerate().rev() {
		// Sum tokens from this cut point to the next cut point (or end).
		let next_boundary = if vi + 1 < valid_points.len() {
			valid_points[vi + 1]
		} else {
			end
		};

		for entry in &entries[cut_idx..next_boundary] {
			tokens_from_end += estimate_entry_tokens(entry);
		}

		// If we've accumulated enough tokens to keep and the cut point
		// isn't at the start (so there's something to summarize):
		if tokens_from_end >= keep_recent_tokens && cut_idx > start {
			return Some(CutPointResult { first_kept_index: cut_idx });
		}
	}

	None
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ai::types::{AssistantMessage, ContentBlock, UserMessage};
	use crate::session::types::SessionMessageEntry;

	fn ts() -> String {
		"2026-01-01T00:00:00Z".to_owned()
	}

	fn user_entry(id: &str, parent: Option<&str>, text: &str) -> SessionEntry {
		SessionEntry::Message(SessionMessageEntry {
			id:        id.to_owned(),
			parent_id: parent.map(|s| s.to_owned()),
			timestamp: ts(),
			message:   Message::User(UserMessage { content: text.to_owned() }),
		})
	}

	fn assistant_entry(id: &str, parent: Option<&str>, text: &str) -> SessionEntry {
		SessionEntry::Message(SessionMessageEntry {
			id:        id.to_owned(),
			parent_id: parent.map(|s| s.to_owned()),
			timestamp: ts(),
			message:   Message::Assistant(AssistantMessage {
				content:     vec![ContentBlock::Text { text: text.to_owned() }],
				stop_reason: None,
				usage:       None,
			}),
		})
	}

	fn tool_result_entry(id: &str, parent: Option<&str>) -> SessionEntry {
		SessionEntry::Message(SessionMessageEntry {
			id:        id.to_owned(),
			parent_id: parent.map(|s| s.to_owned()),
			timestamp: ts(),
			message:   Message::ToolResult(crate::ai::types::ToolResultMessage {
				tool_use_id: "t1".to_owned(),
				content:     std::sync::Arc::new("result".to_owned()),
				is_error:    false,
			}),
		})
	}

	#[test]
	fn valid_cut_points_excludes_tool_results() {
		let e1 = user_entry("a1", None, "hello");
		let e2 = assistant_entry("a2", Some("a1"), "response");
		let e3 = tool_result_entry("a3", Some("a2"));
		let e4 = user_entry("a4", Some("a3"), "follow up");

		let entries: Vec<&SessionEntry> = vec![&e1, &e2, &e3, &e4];
		let valid = find_valid_cut_points(&entries, 0, 4);
		// e3 (tool result) should be excluded
		assert_eq!(valid, vec![0, 1, 3]);
	}

	#[test]
	fn find_cut_point_basic() {
		// Create entries with enough tokens to trigger a cut.
		// Each 400-char message ≈ 100 tokens.
		let long_text = "x".repeat(400);
		let e1 = user_entry("a1", None, &long_text);
		let e2 = assistant_entry("a2", Some("a1"), &long_text);
		let e3 = user_entry("a3", Some("a2"), &long_text);
		let e4 = assistant_entry("a4", Some("a3"), &long_text);
		let e5 = user_entry("a5", Some("a4"), &long_text);
		let e6 = assistant_entry("a6", Some("a5"), &long_text);

		let entries: Vec<&SessionEntry> = vec![&e1, &e2, &e3, &e4, &e5, &e6];

		// keep_recent_tokens = 200, each entry ≈ 100 tokens.
		// Walking backwards: e6(100) + e5(100) = 200 → cut at e5 (index 4)
		let result = find_cut_point(&entries, 0, 6, 200);
		assert!(result.is_some());
		let cut = result.unwrap();
		assert!(cut.first_kept_index > 0, "Should summarize some entries");
		assert!(cut.first_kept_index < 6, "Should keep some entries");
	}

	#[test]
	fn find_cut_point_not_enough_content() {
		let e1 = user_entry("a1", None, "short");
		let e2 = assistant_entry("a2", Some("a1"), "also short");

		let entries: Vec<&SessionEntry> = vec![&e1, &e2];

		// keep_recent_tokens = 1000 but we only have ~5 tokens total
		let result = find_cut_point(&entries, 0, 2, 1000);
		assert!(result.is_none());
	}

	#[test]
	fn find_cut_point_empty() {
		let entries: Vec<&SessionEntry> = vec![];
		let result = find_cut_point(&entries, 0, 0, 100);
		assert!(result.is_none());
	}
}
