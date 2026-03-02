use std::{collections::HashMap, path::Path, sync::Arc};

use tokio_util::sync::CancellationToken;

use crate::{
	tools::{Concurrency, OnToolUpdate, Tool, ToolOutput},
	types::ToolDefinition,
};

/// Builder for assembling a [`ToolRegistry`].
///
/// Register tools on the builder, then call [`.build()`](Self::build) to
/// produce an immutable, cheaply cloneable registry.
pub struct ToolRegistryBuilder {
	tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistryBuilder {
	#[must_use]
	pub fn new() -> Self {
		Self { tools: HashMap::new() }
	}

	/// Register a tool. Returns `&mut Self` for chaining.
	pub fn register(&mut self, tool: Box<dyn Tool>) -> &mut Self {
		self.tools.insert(tool.name().to_owned(), tool);
		self
	}

	/// Freeze the builder into an immutable [`ToolRegistry`].
	#[must_use]
	pub fn build(self) -> ToolRegistry {
		ToolRegistry { tools: Arc::new(self.tools) }
	}
}

impl Default for ToolRegistryBuilder {
	fn default() -> Self {
		Self::new()
	}
}

/// Immutable, cheaply cloneable registry of available tools.
pub struct ToolRegistry {
	tools: Arc<HashMap<String, Box<dyn Tool>>>,
}

impl ToolRegistry {
	/// Create an empty registry (convenience shorthand for
	/// `ToolRegistryBuilder::new().build()`).
	#[must_use]
	pub fn new() -> Self {
		Self { tools: Arc::new(HashMap::new()) }
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

	/// Returns the concurrency mode for a tool by name.
	///
	/// Defaults to [`Concurrency::Exclusive`] for unknown tools (safe
	/// fallback).
	#[must_use]
	pub fn concurrency(&self, name: &str) -> Concurrency {
		self
			.tools
			.get(name)
			.map_or(Concurrency::Exclusive, |t| t.concurrency())
	}

	pub async fn execute(
		&self,
		name: &str,
		input: &serde_json::Value,
		cwd: &Path,
		cancel: &CancellationToken,
		on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput> {
		let tool = self
			.tools
			.get(name)
			.ok_or_else(|| anyhow::anyhow!("Unknown tool: {name}"))?;
		tool.execute(input, cwd, cancel, on_update).await
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
