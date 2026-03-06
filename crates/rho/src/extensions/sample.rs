//! Sample extension for integration testing and as a template.
//!
//! Gated behind `#[cfg(test)]` — not included in release builds.

use std::sync::Arc;

use rho_agent::{
	hooks::{AgentHooks, ToolCallAction},
	tools::Tool,
};

use super::types::Extension;

/// A sample extension that demonstrates the extension API:
/// - Registers an `echo_tool` that returns its input
/// - Provides context ("Sample extension loaded")
/// - Hooks `before_tool_call` to log tool invocations
pub struct SampleExtension;

impl Extension for SampleExtension {
	fn id(&self) -> &str {
		"sample"
	}

	fn name(&self) -> &str {
		"Sample Extension"
	}

	fn tools(&self) -> Vec<Box<dyn Tool>> {
		vec![Box::new(EchoTool)]
	}

	fn hooks(&self) -> Option<Arc<dyn AgentHooks>> {
		Some(Arc::new(SampleHooks))
	}

	fn context_provider(&self) -> Option<String> {
		Some("Sample extension loaded".to_owned())
	}
}

/// A dummy tool that echoes its input back as JSON.
struct EchoTool;

#[async_trait::async_trait]
impl Tool for EchoTool {
	fn name(&self) -> &str {
		"echo_tool"
	}

	fn description(&self) -> &str {
		"Echoes the input back as output (test tool)"
	}

	fn input_schema(&self) -> serde_json::Value {
		serde_json::json!({
			"type": "object",
			"properties": {
				"message": { "type": "string", "description": "The message to echo" }
			},
			"required": ["message"]
		})
	}

	async fn execute(
		&self,
		input: &serde_json::Value,
		_cwd: &std::path::Path,
		_cancel: &tokio_util::sync::CancellationToken,
		_on_update: Option<&rho_agent::tools::OnToolUpdate>,
	) -> anyhow::Result<rho_agent::tools::ToolOutput> {
		let message = input.get("message").and_then(|v| v.as_str()).unwrap_or("");
		Ok(rho_agent::tools::ToolOutput { content: message.to_owned(), is_error: false })
	}
}

/// Sample hooks that log tool invocations.
struct SampleHooks;

#[async_trait::async_trait]
impl AgentHooks for SampleHooks {
	async fn before_tool_call(
		&self,
		name: &str,
		id: &str,
		_input: &serde_json::Value,
	) -> anyhow::Result<ToolCallAction> {
		eprintln!("[sample-ext] before_tool_call: {name} (id={id})");
		Ok(ToolCallAction::Continue)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::extensions::ExtensionManager;

	#[test]
	fn sample_extension_provides_tools() {
		let ext = SampleExtension;
		let tools = ext.tools();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].name(), "echo_tool");
	}

	#[test]
	fn sample_extension_provides_context() {
		let ext = SampleExtension;
		assert_eq!(ext.context_provider().unwrap(), "Sample extension loaded");
	}

	#[test]
	fn sample_extension_provides_hooks() {
		let ext = SampleExtension;
		assert!(ext.hooks().is_some());
	}

	#[test]
	fn sample_extension_registers_in_manager() {
		let mut mgr = ExtensionManager::new();
		mgr.load(Box::new(SampleExtension));

		let tools = mgr.extension_tools();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].name(), "echo_tool");

		let ctx = mgr.context_strings();
		assert_eq!(ctx, vec!["Sample extension loaded"]);

		assert!(mgr.hooks().is_some());
	}

	#[tokio::test]
	async fn echo_tool_returns_input() {
		let tool = EchoTool;
		let input = serde_json::json!({"message": "hello"});
		let cancel = tokio_util::sync::CancellationToken::new();
		let result = tool
			.execute(&input, std::path::Path::new("/tmp"), &cancel, None)
			.await
			.unwrap();
		assert_eq!(result.content, "hello");
		assert!(!result.is_error);
	}

	#[test]
	fn extension_tools_appear_in_definitions() {
		let mut mgr = ExtensionManager::new();
		mgr.load(Box::new(SampleExtension));

		// Build a registry merging built-in + extension tools.
		let mut builder = crate::tools::registry::ToolRegistryBuilder::new();
		for tool in mgr.extension_tools() {
			builder.register(tool);
		}
		let registry = builder.build();

		let defs = registry.definitions();
		assert!(
			defs.iter().any(|d| d.name == "echo_tool"),
			"echo_tool should appear in tool definitions"
		);
	}
}
