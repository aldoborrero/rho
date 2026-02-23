use std::path::Path;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Tool, ToolOutput};

/// Tool that converts HTML content to Markdown.
pub struct HtmlToMarkdownTool;

#[async_trait]
impl Tool for HtmlToMarkdownTool {
	fn name(&self) -> &'static str {
		"html_to_markdown"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/html_to_markdown.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "html": {
						"type": "string",
						"description": "HTML content to convert to Markdown"
				  },
				  "clean": {
						"type": "boolean",
						"description": "Remove navigation, headers, footers (default: true)"
				  }
			 },
			 "required": ["html"]
		})
	}

	async fn execute(&self, input: Value, _cwd: &Path) -> anyhow::Result<ToolOutput> {
		let html = input
			.get("html")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: html"))?;

		let clean = input.get("clean").and_then(Value::as_bool).unwrap_or(true);

		let html_owned = html.to_owned();
		let result = tokio::task::spawn_blocking(move || {
			rho_tools::html::html_to_markdown(
				&html_owned,
				Some(rho_tools::html::HtmlToMarkdownOptions {
					clean_content: Some(clean),
					skip_images:   None,
				}),
			)
		})
		.await
		.map_err(|e| anyhow::anyhow!("HTML to Markdown task panicked: {e}"))?;

		match result {
			Ok(markdown) => Ok(ToolOutput { content: markdown, is_error: false }),
			Err(e) => {
				Ok(ToolOutput { content: format!("HTML to Markdown error: {e}"), is_error: true })
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_html_to_markdown_basic() {
		let tool = HtmlToMarkdownTool;
		let result = tool
			.execute(json!({"html": "<h1>Hello</h1><p>World</p>"}), Path::new("/"))
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(result.content.contains("Hello"), "Expected 'Hello' in: {}", result.content);
		assert!(result.content.contains("World"), "Expected 'World' in: {}", result.content);
	}

	#[tokio::test]
	async fn test_html_to_markdown_missing_html() {
		let tool = HtmlToMarkdownTool;
		let result = tool.execute(json!({}), Path::new("/")).await;
		assert!(result.is_err(), "Expected error for missing html parameter");
	}
}
