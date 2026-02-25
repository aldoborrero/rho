use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Tool, ToolOutput};

/// Default maximum number of output lines.
const DEFAULT_LIMIT: usize = 100;

/// Tool that searches file contents using the rho-tools grep engine.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
	fn name(&self) -> &'static str {
		"grep"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/grep.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "pattern": {
						"type": "string",
						"description": "The regex pattern to search for"
				  },
				  "path": {
						"type": "string",
						"description": "Directory or file to search in (default: cwd)"
				  },
				  "glob": {
						"type": "string",
						"description": "Glob pattern to filter files (e.g. \"*.rs\")"
				  },
				  "type": {
						"type": "string",
						"description": "File type filter (e.g. \"rs\", \"js\", \"py\")"
				  },
				  "i": {
						"type": "boolean",
						"description": "Case-insensitive search"
				  },
				  "multiline": {
						"type": "boolean",
						"description": "Enable multiline matching"
				  },
				  "context": {
						"type": "integer",
						"description": "Lines of context before and after each match"
				  },
				  "limit": {
						"type": "integer",
						"description": "Maximum number of matches to return (default: 100)"
				  }
			 },
			 "required": ["pattern"]
		})
	}

	async fn execute(&self, input: Value, cwd: &Path, cancel: &CancellationToken) -> anyhow::Result<ToolOutput> {
		let pattern = input
			.get("pattern")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

		let search_path = input
			.get("path")
			.and_then(Value::as_str)
			.map_or_else(|| cwd.to_string_lossy().into_owned(), ToString::to_string);

		let limit = input
			.get("limit")
			.and_then(Value::as_u64)
			.map_or(DEFAULT_LIMIT, |v| v as usize);

		let options = rho_tools::grep::GrepOptions {
			pattern:        pattern.to_owned(),
			path:           search_path,
			glob:           input
				.get("glob")
				.and_then(Value::as_str)
				.map(ToOwned::to_owned),
			type_filter:    input
				.get("type")
				.and_then(Value::as_str)
				.map(ToOwned::to_owned),
			ignore_case:    input.get("i").and_then(Value::as_bool),
			multiline:      input.get("multiline").and_then(Value::as_bool),
			hidden:         Some(true),
			cache:          Some(false),
			max_count:      Some(limit as u32),
			offset:         None,
			context_before: input
				.get("context")
				.and_then(Value::as_u64)
				.map(|v| v as u32),
			context_after:  input
				.get("context")
				.and_then(Value::as_u64)
				.map(|v| v as u32),
			context:        None,
			max_columns:    Some(2000),
			mode:           Some("content".to_owned()),
		};

		// Run in a blocking task since rho-tools grep is synchronous.
		let mut ct = rho_tools::cancel::CancelToken::new(Some(30_000));
		let internal_abort = ct.emplace_abort_token();

		// Bridge: external CancellationToken → internal CancelToken.
		let external = cancel.clone();
		let bridge = tokio::spawn(async move {
			external.cancelled().await;
			internal_abort.abort(rho_tools::cancel::AbortReason::Signal);
		});

		let result = tokio::task::spawn_blocking(move || rho_tools::grep::grep(options, None, ct))
			.await
			.map_err(|e| anyhow::anyhow!("Grep task panicked: {e}"))?;
		bridge.abort();

		match result {
			Ok(grep_result) => {
				let mut output = String::new();
				for m in &grep_result.matches {
					if m.line_number > 0 {
						if let Some(ref ctx_before) = m.context_before {
							for ctx in ctx_before {
								let _ = writeln!(output, "{}:{}: {}", m.path, ctx.line_number, ctx.line);
							}
						}
						let _ = writeln!(output, "{}:{}:{}", m.path, m.line_number, m.line);
						if let Some(ref ctx_after) = m.context_after {
							for ctx in ctx_after {
								let _ = writeln!(output, "{}:{}: {}", m.path, ctx.line_number, ctx.line);
							}
						}
					}
				}

				// Trim trailing newline
				let output = output.trim_end().to_owned();

				if output.is_empty() {
					return Ok(ToolOutput { content: "No matches found.".to_owned(), is_error: false });
				}

				let line_count = output.lines().count();
				let mut result_text = if line_count > limit {
					let truncated: String = output.lines().take(limit).collect::<Vec<_>>().join("\n");
					truncated
				} else {
					output
				};

				if line_count > limit {
					let _ = write!(result_text, "\n... (results truncated to {limit} lines)");
				}

				Ok(ToolOutput { content: result_text, is_error: false })
			},
			Err(e) => Ok(ToolOutput { content: format!("Grep error: {e}"), is_error: true }),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::fs;

	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_grep_find_pattern() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("test.txt"), "hello world\nfoo bar\nhello again").unwrap();

		let tool = GrepTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"pattern": "hello", "path": dir.path().to_str().unwrap()}), Path::new("/"), &ct)
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(
			result.content.contains("hello world"),
			"Expected 'hello world' in: {}",
			result.content
		);
		assert!(
			result.content.contains("hello again"),
			"Expected 'hello again' in: {}",
			result.content
		);
		assert!(!result.content.contains("foo bar"), "Unexpected 'foo bar' in: {}", result.content);
	}

	#[tokio::test]
	async fn test_grep_no_matches() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("test.txt"), "hello world").unwrap();

		let tool = GrepTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				json!({"pattern": "zzzznotfound", "path": dir.path().to_str().unwrap()}),
				Path::new("/"),
				&ct,
			)
			.await
			.unwrap();
		assert!(!result.is_error);
		assert!(result.content.contains("No matches found"));
	}

	#[tokio::test]
	async fn test_grep_case_insensitive() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("test.txt"), "Hello World").unwrap();

		let tool = GrepTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				json!({"pattern": "hello", "path": dir.path().to_str().unwrap(), "i": true}),
				Path::new("/"),
				&ct,
			)
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(
			result.content.contains("Hello World"),
			"Expected 'Hello World' in: {}",
			result.content
		);
	}

	#[tokio::test]
	async fn test_grep_with_glob() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
		fs::write(dir.path().join("b.txt"), "fn main() {}").unwrap();

		let tool = GrepTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				json!({"pattern": "fn main", "path": dir.path().to_str().unwrap(), "glob": "*.rs"}),
				Path::new("/"),
				&ct,
			)
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(result.content.contains("a.rs"), "Expected 'a.rs' in: {}", result.content);
		assert!(!result.content.contains("b.txt"), "Unexpected 'b.txt' in: {}", result.content);
	}
}
