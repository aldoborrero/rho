use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;
use tokio_util::sync::CancellationToken;

use super::{OnToolUpdate, Tool, ToolOutput};

/// Default maximum number of lines to read.
const DEFAULT_LIMIT: usize = 2000;

/// Tool that reads file contents with optional offset and limit.
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
	fn name(&self) -> &str {
		"read"
	}

	fn description(&self) -> &str {
		include_str!("../prompts/tools/read.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "path": {
						"type": "string",
						"description": "Absolute or relative path to the file or directory"
				  },
				  "offset": {
						"type": "integer",
						"description": "1-indexed line number to start reading from"
				  },
				  "limit": {
						"type": "integer",
						"description": "Maximum number of lines to read (default: 2000)"
				  }
			 },
			 "required": ["path"]
		})
	}

	async fn execute(
		&self,
		input: &Value,
		cwd: &Path,
		_cancel: &CancellationToken,
		_on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput> {
		let raw_path = input
			.get("path")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

		let path = cwd.join(raw_path);

		// Check if the path is a directory.
		match fs::metadata(&path).await {
			Ok(meta) if meta.is_dir() => {
				let mut entries = fs::read_dir(&path).await?;
				let mut listing = Vec::new();
				while let Some(entry) = entries.next_entry().await? {
					let name = entry.file_name().to_string_lossy().into_owned();
					let file_type = entry.file_type().await?;
					let suffix = if file_type.is_dir() { "/" } else { "" };
					listing.push(format!("{name}{suffix}"));
				}
				listing.sort();
				return Ok(ToolOutput { content: listing.join("\n"), is_error: false });
			},
			Ok(_) => {}, // regular file, continue below
			Err(e) => {
				return Ok(ToolOutput {
					content:  format!("Error reading {}: {e}", path.display()),
					is_error: true,
				});
			},
		}

		let content = match fs::read_to_string(&path).await {
			Ok(c) => c,
			Err(e) => {
				return Ok(ToolOutput {
					content:  format!("Error reading {}: {e}", path.display()),
					is_error: true,
				});
			},
		};

		let offset = input
			.get("offset")
			.and_then(Value::as_u64)
			.map_or(1, |v| v.max(1)) as usize;

		let limit = input
			.get("limit")
			.and_then(Value::as_u64)
			.map_or(DEFAULT_LIMIT, |v| v as usize);

		let lines: Vec<&str> = content.lines().collect();
		let start = offset.saturating_sub(1); // convert from 1-indexed to 0-indexed
		let end = lines.len().min(start + limit);

		let mut result = String::new();
		for (idx, line) in lines[start..end].iter().enumerate() {
			let line_num = start + idx + 1;
			let _ = writeln!(result, "{line_num:>6}\t{line}");
		}

		Ok(ToolOutput { content: result, is_error: false })
	}
}

#[cfg(test)]
mod tests {
	use std::io::Write as _;

	use tempfile::NamedTempFile;
	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_read_file() {
		let mut tmp = NamedTempFile::new().unwrap();
		writeln!(tmp, "line one").unwrap();
		writeln!(tmp, "line two").unwrap();
		writeln!(tmp, "line three").unwrap();

		let tool = ReadTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(&json!({"path": tmp.path().to_str().unwrap()}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		assert!(!result.is_error);
		assert!(result.content.contains("line one"));
		assert!(result.content.contains("line three"));
		// Check line numbers are present
		assert!(result.content.contains("1\t"));
		assert!(result.content.contains("3\t"));
	}

	#[tokio::test]
	async fn test_read_with_offset_and_limit() {
		let mut tmp = NamedTempFile::new().unwrap();
		for i in 1..=10 {
			writeln!(tmp, "line {i}").unwrap();
		}

		let tool = ReadTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				&json!({"path": tmp.path().to_str().unwrap(), "offset": 3, "limit": 2}),
				Path::new("/"),
				&ct,
				None,
			)
			.await
			.unwrap();
		assert!(!result.is_error);
		assert!(result.content.contains("line 3"));
		assert!(result.content.contains("line 4"));
		assert!(!result.content.contains("line 5"));
		assert!(!result.content.contains("line 2"));
	}

	#[tokio::test]
	async fn test_read_nonexistent_file() {
		let tool = ReadTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(&json!({"path": "/tmp/nonexistent_file_abc123xyz"}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		assert!(result.is_error);
	}

	#[tokio::test]
	async fn test_read_directory() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
		std::fs::write(dir.path().join("b.txt"), "world").unwrap();

		let tool = ReadTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(&json!({"path": dir.path().to_str().unwrap()}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		assert!(!result.is_error);
		assert!(result.content.contains("a.txt"));
		assert!(result.content.contains("b.txt"));
	}
}
