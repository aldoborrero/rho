//! `/help` and `/hotkeys` command handlers.

use std::fmt::Write as _;

use super::super::{registry::COMMANDS, types::CommandResult};

/// Handler for `/help` — list all available slash commands.
pub fn cmd_help() -> CommandResult {
	let mut text = String::from("Available commands:\n\n");
	for cmd in COMMANDS {
		let _ = write!(text, "  /{}", cmd.name);
		if let Some(hint) = cmd.args_hint {
			let _ = write!(text, " {hint}");
		}
		if !cmd.aliases.is_empty() {
			let aliases: Vec<String> = cmd.aliases.iter().map(|a| format!("/{a}")).collect();
			let _ = write!(text, "  (aliases: {})", aliases.join(", "));
		}
		let _ = writeln!(text, "\n    {}", cmd.description);
	}
	text.push_str("\nPrefix a command with ! to run it in the shell (e.g. !ls).");
	CommandResult::Message(text)
}

/// Handler for `/hotkeys` — show keyboard shortcuts.
pub fn cmd_hotkeys() -> CommandResult {
	let text = "\
Keyboard shortcuts:

  Enter        Submit message
  Ctrl+C       Cancel current stream / exit
  Ctrl+D       Exit
  Ctrl+L       Clear chat display
  Up/Down      Scroll chat history
  Esc          Clear editor";
	CommandResult::Message(text.to_owned())
}
