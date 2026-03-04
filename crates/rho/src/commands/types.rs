//! Types for the slash command system.

use crate::{session::SessionManager, settings::Settings, tools::registry::ToolRegistry};

/// Result of executing a slash command.
///
/// Each variant declares intent — the event loop performs the effect.
pub enum CommandResult {
	/// Display a message in chat (not sent to AI).
	Message(String),
	/// Exit the application.
	Exit,
	/// Clear chat and start a new session.
	NewSession,
	/// Change working directory.
	ChangeDir(String),
	/// Fork the current session. Event loop calls `session.fork()`.
	Fork,
	/// Trigger conversation compaction (optional focus instructions).
	Compact(Option<String>),
	/// Change the active model.
	ModelChange(String),
	/// Settings were changed on disk; event loop should reload.
	SettingsChanged,
	/// Open the interactive model selector overlay.
	ShowModelSelector,
	/// No visible output (e.g., clipboard operation already done).
	Silent,
}

/// Subcommand metadata (for autocomplete, not dispatch).
pub struct SubcommandDef {
	pub name:        &'static str,
	pub description: &'static str,
}

/// Metadata for a registered slash command.
pub struct SlashCommand {
	pub name:        &'static str,
	pub aliases:     &'static [&'static str],
	pub description: &'static str,
	pub args_hint:   Option<&'static str>,
	pub subcommands: &'static [SubcommandDef],
}

/// Read-only context provided to slash command handlers.
///
/// Borrows from the event loop's owned state. Commands read but never mutate.
pub struct CommandContext<'a> {
	pub name:     &'a str,
	pub args:     &'a str,
	pub session:  &'a SessionManager,
	pub settings: &'a Settings,
	pub model:    &'a str,
	pub tools:    &'a ToolRegistry,
}
