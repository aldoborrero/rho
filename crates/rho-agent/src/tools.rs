use std::path::Path;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// Output from executing a tool.
pub struct ToolOutput {
	pub content:  String,
	pub is_error: bool,
}

/// Trait for tools that the AI can invoke.
#[async_trait]
pub trait Tool: Send + Sync {
	/// The tool name (used in API calls).
	fn name(&self) -> &'static str;

	/// Human-readable description of the tool.
	fn description(&self) -> &'static str;

	/// JSON Schema for the tool's input.
	fn input_schema(&self) -> serde_json::Value;

	/// Execute the tool with the given input.
	async fn execute(
		&self,
		input: serde_json::Value,
		cwd: &Path,
		cancel: &CancellationToken,
	) -> anyhow::Result<ToolOutput>;
}
