use std::sync::Arc;

use rho_agent::{hooks::AgentHooks, tools::Tool};

use crate::commands::CommandResult;

/// Manifest from `extension.toml`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExtensionManifest {
	pub id:          String,
	pub name:        String,
	pub version:     String,
	#[serde(default)]
	pub description: String,
}

/// A dynamically registered slash command.
pub struct DynamicCommand {
	pub name:        String,
	pub aliases:     Vec<String>,
	pub description: String,
	pub handler:     Box<dyn Fn(&str) -> anyhow::Result<CommandResult> + Send + Sync>,
}

/// Extension trait — provides tools, commands, hooks, and context.
pub trait Extension: Send + Sync {
	fn id(&self) -> &str;
	fn name(&self) -> &str;

	fn tools(&self) -> Vec<Box<dyn Tool>> {
		Vec::new()
	}

	fn commands(&self) -> Vec<DynamicCommand> {
		Vec::new()
	}

	fn hooks(&self) -> Option<Arc<dyn AgentHooks>> {
		None
	}

	/// Additional text injected into the system prompt.
	fn context_provider(&self) -> Option<String> {
		None
	}
}
