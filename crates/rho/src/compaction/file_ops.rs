//! File operation tracking for compaction summaries.
//!
//! Extracts which files were read/written/edited from tool call blocks
//! in assistant messages.
//!
//! oh-my-pi ref: `compaction/utils.ts` `FileOperations`,
//! `extractFileOpsFromMessage()`, `computeFileLists()`

use std::collections::HashSet;

use crate::ai::types::{ContentBlock, Message};

/// Maximum files to include in summary XML tags.
const FILE_LIST_LIMIT: usize = 20;

/// Tracked file operations during a conversation segment.
pub struct FileOperations {
	pub read:    HashSet<String>,
	pub written: HashSet<String>,
	pub edited:  HashSet<String>,
}

/// Extract file operations from assistant tool call blocks.
///
/// Scans for `ToolUse` blocks with read/write/edit tool names and extracts
/// the `"path"` or `"file_path"` argument.
pub fn extract_file_ops(messages: &[Message]) -> FileOperations {
	let mut ops = FileOperations {
		read:    HashSet::new(),
		written: HashSet::new(),
		edited:  HashSet::new(),
	};

	for message in messages {
		if let Message::Assistant(a) = message {
			for block in &a.content {
				if let ContentBlock::ToolUse { name, input, .. } = block {
					let path = input
						.get("path")
						.or_else(|| input.get("file_path"))
						.and_then(|v| v.as_str())
						.map(String::from);

					if let Some(path) = path {
						match name.as_str() {
							"read" | "Read" => {
								ops.read.insert(path);
							},
							"write" | "Write" => {
								ops.written.insert(path);
							},
							"edit" | "Edit" => {
								ops.edited.insert(path);
							},
							_ => {},
						}
					}
				}
			}
		}
	}

	ops
}

/// Compute final file lists: `modified = written ∪ edited`,
/// `read_only = read - modified`.
///
/// Returns `(read_only_files, modified_files)`, sorted and truncated to
/// [`FILE_LIST_LIMIT`].
pub fn compute_file_lists(ops: &FileOperations) -> (Vec<String>, Vec<String>) {
	let modified: HashSet<&String> = ops.written.union(&ops.edited).collect();

	let mut read_only: Vec<String> = ops
		.read
		.iter()
		.filter(|f| !modified.contains(f))
		.cloned()
		.collect();
	read_only.sort();
	read_only.truncate(FILE_LIST_LIMIT);

	let mut modified_list: Vec<String> = modified.into_iter().cloned().collect();
	modified_list.sort();
	modified_list.truncate(FILE_LIST_LIMIT);

	(read_only, modified_list)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ai::types::AssistantMessage;

	fn tool_use_msg(name: &str, path: &str) -> Message {
		Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "t1".to_owned(),
				name:  name.to_owned(),
				input: serde_json::json!({"path": path}),
			}],
			stop_reason: None,
			usage:       None,
		})
	}

	#[test]
	fn extract_read_ops() {
		let msgs = vec![tool_use_msg("Read", "src/main.rs")];
		let ops = extract_file_ops(&msgs);
		assert!(ops.read.contains("src/main.rs"));
		assert!(ops.written.is_empty());
		assert!(ops.edited.is_empty());
	}

	#[test]
	fn extract_write_ops() {
		let msgs = vec![tool_use_msg("Write", "src/lib.rs")];
		let ops = extract_file_ops(&msgs);
		assert!(ops.written.contains("src/lib.rs"));
	}

	#[test]
	fn extract_edit_ops() {
		let msgs = vec![tool_use_msg("Edit", "src/lib.rs")];
		let ops = extract_file_ops(&msgs);
		assert!(ops.edited.contains("src/lib.rs"));
	}

	#[test]
	fn extract_file_path_arg() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "t1".to_owned(),
				name:  "read".to_owned(),
				input: serde_json::json!({"file_path": "/tmp/test.txt"}),
			}],
			stop_reason: None,
			usage:       None,
		});
		let ops = extract_file_ops(&[msg]);
		assert!(ops.read.contains("/tmp/test.txt"));
	}

	#[test]
	fn compute_file_lists_separates_read_from_modified() {
		let ops = FileOperations {
			read:    HashSet::from(["a.rs".to_owned(), "b.rs".to_owned(), "c.rs".to_owned()]),
			written: HashSet::from(["b.rs".to_owned()]),
			edited:  HashSet::from(["c.rs".to_owned()]),
		};
		let (read_only, modified) = compute_file_lists(&ops);
		assert_eq!(read_only, vec!["a.rs"]);
		assert!(modified.contains(&"b.rs".to_owned()));
		assert!(modified.contains(&"c.rs".to_owned()));
	}

	#[test]
	fn compute_file_lists_empty() {
		let ops = FileOperations {
			read:    HashSet::new(),
			written: HashSet::new(),
			edited:  HashSet::new(),
		};
		let (read_only, modified) = compute_file_lists(&ops);
		assert!(read_only.is_empty());
		assert!(modified.is_empty());
	}

	#[test]
	fn ignores_non_file_tools() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "t1".to_owned(),
				name:  "bash".to_owned(),
				input: serde_json::json!({"command": "ls"}),
			}],
			stop_reason: None,
			usage:       None,
		});
		let ops = extract_file_ops(&[msg]);
		assert!(ops.read.is_empty());
		assert!(ops.written.is_empty());
		assert!(ops.edited.is_empty());
	}
}
