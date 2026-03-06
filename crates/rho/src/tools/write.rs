use std::path::Path;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::fs;
use tokio_util::sync::CancellationToken;

use super::{Concurrency, OnToolUpdate, Tool, ToolOutput};

/// Tool that writes content to a file.
pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
	fn name(&self) -> &'static str {
		"write"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/write.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "path": {
						"type": "string",
						"description": "Absolute or relative path to the file"
				  },
				  "content": {
						"type": "string",
						"description": "Content to write to the file"
				  }
			 },
			 "required": ["path", "content"]
		})
	}

	fn concurrency(&self) -> Concurrency {
		Concurrency::Exclusive
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

		let content = input
			.get("content")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: content"))?;

		let path = cwd.join(raw_path);

		// Create parent directories if they don't exist.
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).await?;
		}

		fs::write(&path, content).await?;

		let bytes_written = content.len();
		Ok(ToolOutput {
			content:  format!("Wrote {bytes_written} bytes to {}", path.display()),
			is_error: false,
		})
	}
}

#[cfg(test)]
mod tests {
	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_write_file() {
		let dir = tempfile::tempdir().unwrap();
		let file_path = dir.path().join("test.txt");

		let tool = WriteTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				&json!({"path": file_path.to_str().unwrap(), "content": "hello world"}),
				Path::new("/"),
				&ct,
				None,
			)
			.await
			.unwrap();
		assert!(!result.is_error);

		let content = std::fs::read_to_string(&file_path).unwrap();
		assert_eq!(content, "hello world");
	}

	#[tokio::test]
	async fn test_write_creates_nested_dirs() {
		let dir = tempfile::tempdir().unwrap();
		let file_path = dir.path().join("a").join("b").join("c").join("test.txt");

		let tool = WriteTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				&json!({"path": file_path.to_str().unwrap(), "content": "nested"}),
				Path::new("/"),
				&ct,
				None,
			)
			.await
			.unwrap();
		assert!(!result.is_error);

		let content = std::fs::read_to_string(&file_path).unwrap();
		assert_eq!(content, "nested");
	}

	#[tokio::test]
	async fn test_write_overwrites_existing() {
		let dir = tempfile::tempdir().unwrap();
		let file_path = dir.path().join("overwrite.txt");
		std::fs::write(&file_path, "old content").unwrap();

		let tool = WriteTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(
				&json!({"path": file_path.to_str().unwrap(), "content": "new content"}),
				Path::new("/"),
				&ct,
				None,
			)
			.await
			.unwrap();
		assert!(!result.is_error);

		let content = std::fs::read_to_string(&file_path).unwrap();
		assert_eq!(content, "new content");
	}
}
