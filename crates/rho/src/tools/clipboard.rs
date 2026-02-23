use std::path::Path;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Tool, ToolOutput};

/// Tool that copies text to the system clipboard.
pub struct ClipboardTool;

#[async_trait]
impl Tool for ClipboardTool {
	fn name(&self) -> &'static str {
		"clipboard"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/clipboard.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "text": {
						"type": "string",
						"description": "Text to copy to the clipboard"
				  }
			 },
			 "required": ["text"]
		})
	}

	async fn execute(&self, input: Value, _cwd: &Path) -> anyhow::Result<ToolOutput> {
		let text = input
			.get("text")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: text"))?;

		let text_owned = text.to_owned();
		let result =
			tokio::task::spawn_blocking(move || rho_tools::clipboard::copy_to_clipboard(text_owned))
				.await
				.map_err(|e| anyhow::anyhow!("Clipboard task panicked: {e}"))?;

		match result {
			Ok(()) => {
				Ok(ToolOutput { content: "Text copied to clipboard.".to_owned(), is_error: false })
			},
			Err(e) => Ok(ToolOutput { content: format!("Clipboard error: {e}"), is_error: true }),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_clipboard_missing_text() {
		let tool = ClipboardTool;
		let result = tool.execute(json!({}), Path::new("/")).await;
		assert!(result.is_err(), "Expected error for missing text parameter");
	}
}
