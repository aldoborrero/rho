//! Compaction prompt templates.
//!
//! All templates are embedded at compile time via `include_str!()`.

use minijinja::Environment;
use serde::Serialize;

/// System prompt for the summarization LLM call.
pub const SUMMARIZATION_SYSTEM: &str = include_str!("summarization-system.md");

/// Initial summary prompt (no previous summary exists).
pub const SUMMARY_PROMPT: &str = include_str!("compaction-summary.md");

/// Iterative update prompt (previous summary exists).
pub const UPDATE_SUMMARY_PROMPT: &str = include_str!("compaction-update-summary.md");

/// Short PR-style summary (2-3 sentences).
pub const SHORT_SUMMARY_PROMPT: &str = include_str!("compaction-short-summary.md");

/// Turn prefix summary (when splitting mid-turn).
pub const TURN_PREFIX_PROMPT: &str = include_str!("compaction-turn-prefix.md");

/// Preamble template for injecting a compaction summary into the conversation.
pub const SUMMARY_CONTEXT: &str = include_str!("compaction-summary-context.md");

/// File operations XML template.
const FILE_OPERATIONS_TEMPLATE: &str = include_str!("file-operations.md");

/// Context for rendering the file-operations template.
#[derive(Serialize)]
struct FileOpsContext {
	read_files:     Vec<String>,
	modified_files: Vec<String>,
}

/// Render the file operations template with read/modified file lists.
///
/// Returns XML tags `<read-files>` and `<modified-files>` if the
/// corresponding lists are non-empty.
pub fn render_file_operations(read_files: &[String], modified_files: &[String]) -> String {
	let mut env = Environment::new();
	env.add_template("file-operations", FILE_OPERATIONS_TEMPLATE)
		.expect("file-operations template is valid");
	let tmpl = env.get_template("file-operations").expect("template registered above");
	let ctx = FileOpsContext {
		read_files:     read_files.to_vec(),
		modified_files: modified_files.to_vec(),
	};
	tmpl.render(&ctx).unwrap_or_default()
}

/// Render the summary context preamble, wrapping the summary in the template.
pub fn render_summary_context(summary: &str) -> String {
	let mut env = Environment::new();
	env.add_template("summary-context", SUMMARY_CONTEXT)
		.expect("summary-context template is valid");
	let tmpl = env.get_template("summary-context").expect("template registered above");
	tmpl.render(minijinja::context! { summary => summary }).unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn templates_are_non_empty() {
		assert!(!SUMMARIZATION_SYSTEM.is_empty());
		assert!(!SUMMARY_PROMPT.is_empty());
		assert!(!UPDATE_SUMMARY_PROMPT.is_empty());
		assert!(!SHORT_SUMMARY_PROMPT.is_empty());
		assert!(!TURN_PREFIX_PROMPT.is_empty());
		assert!(!SUMMARY_CONTEXT.is_empty());
	}

	#[test]
	fn render_file_operations_both() {
		let read = vec!["src/main.rs".to_owned(), "README.md".to_owned()];
		let modified = vec!["src/lib.rs".to_owned()];
		let result = render_file_operations(&read, &modified);
		assert!(result.contains("<read-files>"));
		assert!(result.contains("src/main.rs"));
		assert!(result.contains("<modified-files>"));
		assert!(result.contains("src/lib.rs"));
	}

	#[test]
	fn render_file_operations_empty() {
		let result = render_file_operations(&[], &[]);
		assert!(!result.contains("<read-files>"));
		assert!(!result.contains("<modified-files>"));
	}

	#[test]
	fn render_summary_context_injects_summary() {
		let result = render_summary_context("Test summary content");
		assert!(result.contains("Test summary content"));
		assert!(result.contains("<summary>"));
	}
}
