use std::sync::Arc;

use rho_agent::{
	hooks::{AgentHooks, HookChain},
	tools::Tool,
};

use super::types::{DynamicCommand, Extension};
use crate::commands::CommandResult;

/// Coordinates loaded extensions: collects tools, commands, hooks, and context.
pub struct ExtensionManager {
	extensions:       Vec<Box<dyn Extension>>,
	dynamic_commands: Vec<DynamicCommand>,
	hook_chain:       Option<HookChain>,
}

impl ExtensionManager {
	#[must_use]
	pub fn new() -> Self {
		Self { extensions: Vec::new(), dynamic_commands: Vec::new(), hook_chain: None }
	}

	/// Register an extension, collecting its tools, commands, and hooks.
	pub fn load(&mut self, ext: Box<dyn Extension>) {
		self.dynamic_commands.extend(ext.commands());
		self.extensions.push(ext);
		// Rebuild the hook chain whenever a new extension is loaded.
		self.rebuild_hook_chain();
	}

	/// Collect all tools provided by loaded extensions.
	pub fn extension_tools(&self) -> Vec<Box<dyn Tool>> {
		let mut tools = Vec::new();
		for ext in &self.extensions {
			tools.extend(ext.tools());
		}
		tools
	}

	/// Return a composite hook dispatcher covering all extensions, or `None`
	/// if no extension provides hooks.
	pub fn hooks(&self) -> Option<Arc<dyn AgentHooks>> {
		self.hook_chain.as_ref().map(|_| -> Arc<dyn AgentHooks> {
			// Rebuild a fresh HookChain each time so it captures the
			// current set of extension hooks.
			let hooks: Vec<Arc<dyn AgentHooks>> =
				self.extensions.iter().filter_map(|e| e.hooks()).collect();
			if hooks.is_empty() {
				return Arc::new(HookChain::new(vec![]));
			}
			Arc::new(HookChain::new(hooks))
		})
	}

	/// Try to dispatch a command name to a dynamic extension command.
	/// Returns `None` if no extension handles this command name.
	pub fn dispatch_command(&self, name: &str, args: &str) -> Option<anyhow::Result<CommandResult>> {
		for cmd in &self.dynamic_commands {
			if cmd.name == name || cmd.aliases.iter().any(|a| a == name) {
				return Some((cmd.handler)(args));
			}
		}
		None
	}

	/// Collect context strings from all extensions for system prompt injection.
	pub fn context_strings(&self) -> Vec<String> {
		self
			.extensions
			.iter()
			.filter_map(|e| e.context_provider())
			.collect()
	}

	fn rebuild_hook_chain(&mut self) {
		let hooks: Vec<Arc<dyn AgentHooks>> =
			self.extensions.iter().filter_map(|e| e.hooks()).collect();
		self.hook_chain = if hooks.is_empty() {
			None
		} else {
			Some(HookChain::new(hooks))
		};
	}
}

impl Default for ExtensionManager {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct DummyExtension;

	impl Extension for DummyExtension {
		fn id(&self) -> &str {
			"dummy"
		}

		fn name(&self) -> &str {
			"Dummy"
		}

		fn context_provider(&self) -> Option<String> {
			Some("dummy context".to_owned())
		}
	}

	struct CommandExtension;

	impl Extension for CommandExtension {
		fn id(&self) -> &str {
			"cmd-ext"
		}

		fn name(&self) -> &str {
			"Command Extension"
		}

		fn commands(&self) -> Vec<DynamicCommand> {
			vec![DynamicCommand {
				name:        "greet".to_owned(),
				aliases:     vec!["hello".to_owned()],
				description: "Say hello".to_owned(),
				handler:     Box::new(|args| Ok(CommandResult::Message(format!("Hello, {args}!")))),
			}]
		}
	}

	#[test]
	fn empty_manager_has_no_hooks_or_tools() {
		let mgr = ExtensionManager::new();
		assert!(mgr.hooks().is_none());
		assert!(mgr.extension_tools().is_empty());
		assert!(mgr.context_strings().is_empty());
	}

	#[test]
	fn load_extension_provides_context() {
		let mut mgr = ExtensionManager::new();
		mgr.load(Box::new(DummyExtension));
		let ctx = mgr.context_strings();
		assert_eq!(ctx.len(), 1);
		assert_eq!(ctx[0], "dummy context");
	}

	#[test]
	fn dispatch_command_by_name() {
		let mut mgr = ExtensionManager::new();
		mgr.load(Box::new(CommandExtension));
		let result = mgr.dispatch_command("greet", "world");
		assert!(result.is_some());
		match result.unwrap().unwrap() {
			CommandResult::Message(msg) => assert_eq!(msg, "Hello, world!"),
			_ => panic!("expected Message"),
		}
	}

	#[test]
	fn dispatch_command_by_alias() {
		let mut mgr = ExtensionManager::new();
		mgr.load(Box::new(CommandExtension));
		let result = mgr.dispatch_command("hello", "alias");
		assert!(result.is_some());
	}

	#[test]
	fn dispatch_unknown_command_returns_none() {
		let mut mgr = ExtensionManager::new();
		mgr.load(Box::new(CommandExtension));
		assert!(mgr.dispatch_command("nonexistent", "").is_none());
	}
}
