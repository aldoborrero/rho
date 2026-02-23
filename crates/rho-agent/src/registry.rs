use std::{collections::HashMap, path::Path, sync::Arc};

use crate::{
	tools::{Tool, ToolOutput},
	types::ToolDefinition,
};

/// Registry of available tools.
pub struct ToolRegistry {
	tools: Arc<HashMap<String, Box<dyn Tool>>>,
}

impl ToolRegistry {
	#[must_use]
	pub fn new() -> Self {
		Self { tools: Arc::new(HashMap::new()) }
	}

	pub fn register(&mut self, tool: Box<dyn Tool>) {
		Arc::get_mut(&mut self.tools)
			.expect("Cannot register tools after cloning the registry")
			.insert(tool.name().to_owned(), tool);
	}

	#[must_use]
	pub fn definitions(&self) -> Vec<ToolDefinition> {
		self
			.tools
			.values()
			.map(|t| ToolDefinition {
				name:         t.name().to_owned(),
				description:  t.description().to_owned(),
				input_schema: t.input_schema(),
			})
			.collect()
	}

	pub async fn execute(
		&self,
		name: &str,
		input: serde_json::Value,
		cwd: &Path,
	) -> anyhow::Result<ToolOutput> {
		let tool = self
			.tools
			.get(name)
			.ok_or_else(|| anyhow::anyhow!("Unknown tool: {name}"))?;
		tool.execute(input, cwd).await
	}
}

impl Clone for ToolRegistry {
	fn clone(&self) -> Self {
		Self { tools: Arc::clone(&self.tools) }
	}
}

impl Default for ToolRegistry {
	fn default() -> Self {
		Self::new()
	}
}
