use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Tool, ToolOutput};

/// Default maximum number of fuzzy-find results.
const DEFAULT_MAX_RESULTS: u32 = 20;

/// Tool that finds files using fuzzy matching on file paths.
pub struct FuzzyFindTool;

#[async_trait]
impl Tool for FuzzyFindTool {
	fn name(&self) -> &'static str {
		"fuzzy_find"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/fuzzy_find.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "query": {
						"type": "string",
						"description": "Fuzzy query to match against file paths"
				  },
				  "path": {
						"type": "string",
						"description": "Directory to search in (default: cwd)"
				  },
				  "max_results": {
						"type": "integer",
						"description": "Maximum number of results to return (default: 20)"
				  }
			 },
			 "required": ["query"]
		})
	}

	async fn execute(&self, input: Value, cwd: &Path, _cancel: &CancellationToken) -> anyhow::Result<ToolOutput> {
		let query = input
			.get("query")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

		let search_path = input
			.get("path")
			.and_then(Value::as_str)
			.map_or_else(|| cwd.to_string_lossy().into_owned(), ToString::to_string);

		let max_results = input
			.get("max_results")
			.and_then(Value::as_u64)
			.map_or(DEFAULT_MAX_RESULTS, |v| v as u32);

		let options = rho_tools::fd::FuzzyFindOptions {
			query:       query.to_owned(),
			path:        search_path,
			hidden:      Some(false),
			gitignore:   Some(true),
			cache:       Some(false),
			max_results: Some(max_results),
		};

		let ct = rho_tools::cancel::CancelToken::new(Some(30_000));
		let result = tokio::task::spawn_blocking(move || rho_tools::fd::fuzzy_find(options, ct))
			.await
			.map_err(|e| anyhow::anyhow!("Fuzzy find task panicked: {e}"))?;

		match result {
			Ok(fuzzy_result) => {
				if fuzzy_result.matches.is_empty() {
					return Ok(ToolOutput { content: "No matches found.".to_owned(), is_error: false });
				}

				let mut output = String::new();
				for m in &fuzzy_result.matches {
					let kind = if m.is_directory { "dir" } else { "file" };
					let _ = writeln!(output, "{} (score: {}, {})", m.path, m.score, kind);
				}

				Ok(ToolOutput { content: output.trim_end().to_owned(), is_error: false })
			},
			Err(e) => Ok(ToolOutput { content: format!("Fuzzy find error: {e}"), is_error: true }),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::fs;

	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_fuzzy_find_matches_files() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("controller.rs"), "").unwrap();
		fs::write(dir.path().join("config.toml"), "").unwrap();
		fs::write(dir.path().join("readme.md"), "").unwrap();

		let tool = FuzzyFindTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"query": "ctrl", "path": dir.path().to_str().unwrap()}), Path::new("/"), &ct)
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(
			result.content.contains("controller.rs"),
			"Expected 'controller.rs' in: {}",
			result.content
		);
	}

	#[tokio::test]
	async fn test_fuzzy_find_no_matches() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("hello.txt"), "").unwrap();

		let tool = FuzzyFindTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				json!({"query": "zzznotfound", "path": dir.path().to_str().unwrap()}),
				Path::new("/"),
				&ct,
			)
			.await
			.unwrap();
		assert!(!result.is_error);
		assert!(
			result.content.contains("No matches found"),
			"Expected 'No matches found' in: {}",
			result.content
		);
	}
}
