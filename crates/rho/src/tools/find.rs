use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Tool, ToolOutput};

/// Default maximum number of results.
const DEFAULT_LIMIT: usize = 1000;

/// Tool that finds files by name pattern using the rho-tools glob engine.
pub struct FindTool;

#[async_trait]
impl Tool for FindTool {
	fn name(&self) -> &'static str {
		"find"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/find.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "pattern": {
						"type": "string",
						"description": "Glob pattern to match file names"
				  },
				  "path": {
						"type": "string",
						"description": "Directory to search in (default: cwd)"
				  },
				  "type": {
						"type": "string",
						"description": "Filter by type: \"file\", \"dir\", or \"symlink\""
				  },
				  "limit": {
						"type": "integer",
						"description": "Maximum number of results (default: 1000)"
				  }
			 },
			 "required": ["pattern"]
		})
	}

	async fn execute(&self, input: Value, cwd: &Path) -> anyhow::Result<ToolOutput> {
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

		let file_type = input
			.get("type")
			.and_then(Value::as_str)
			.and_then(|t| match t {
				"file" => Some(rho_tools::glob::FileType::File),
				"dir" => Some(rho_tools::glob::FileType::Dir),
				"symlink" => Some(rho_tools::glob::FileType::Symlink),
				_ => None,
			});

		let pattern_owned = pattern.to_owned();
		let options = rho_tools::glob::GlobOptions {
			pattern: pattern_owned,
			path: search_path,
			file_type,
			recursive: Some(true),
			hidden: Some(false),
			max_results: Some(limit as u32),
			gitignore: Some(true),
			cache: Some(false),
			sort_by_mtime: Some(true),
			include_node_modules: None,
			timeout_ms: Some(30_000),
		};

		// Run in a blocking task since rho-tools glob is synchronous.
		let result = tokio::task::spawn_blocking(move || rho_tools::glob::glob(options, None))
			.await
			.map_err(|e| anyhow::anyhow!("Find task panicked: {e}"))?;

		match result {
			Ok(glob_result) => {
				let lines: Vec<&str> = glob_result
					.matches
					.iter()
					.map(|m| m.path.as_str())
					.take(limit)
					.collect();
				let truncated = glob_result.matches.len() > limit;

				let mut result_text = lines.join("\n");
				if truncated {
					let _ = write!(result_text, "\n... (results truncated to {limit} entries)");
				}

				if result_text.is_empty() {
					"No files found.".clone_into(&mut result_text);
				}

				Ok(ToolOutput { content: result_text, is_error: false })
			},
			Err(e) => {
				Ok(ToolOutput { content: format!("Failed to find files: {e}"), is_error: true })
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	#[tokio::test]
	async fn test_find_files() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("hello.rs"), "").unwrap();
		fs::write(dir.path().join("world.rs"), "").unwrap();
		fs::write(dir.path().join("readme.md"), "").unwrap();

		let tool = FindTool;
		let result = tool
			.execute(json!({"pattern": "*.rs", "path": dir.path().to_str().unwrap()}), Path::new("/"))
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(result.content.contains("hello.rs"), "Expected 'hello.rs' in: {}", result.content);
		assert!(result.content.contains("world.rs"), "Expected 'world.rs' in: {}", result.content);
		assert!(
			!result.content.contains("readme.md"),
			"Unexpected 'readme.md' in: {}",
			result.content
		);
	}

	#[tokio::test]
	async fn test_find_no_results() {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("test.txt"), "").unwrap();

		let tool = FindTool;
		let result = tool
			.execute(
				json!({"pattern": "*.nonexistent", "path": dir.path().to_str().unwrap()}),
				Path::new("/"),
			)
			.await
			.unwrap();
		assert!(!result.is_error);
		assert!(result.content.contains("No files found"));
	}
}
