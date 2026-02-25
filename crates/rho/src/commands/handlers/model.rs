//! `/model` and `/usage` command handlers.

use std::fmt::Write;

use super::super::types::{CommandContext, CommandResult};

/// Handler for `/model` — show or change the active model.
pub fn cmd_model(ctx: &CommandContext<'_>) -> CommandResult {
	if ctx.args.is_empty() {
		let settings = ctx.settings;
		let mut msg = format!("Current model: {}\n\n", ctx.model);
		msg.push_str("Roles:\n");
		let _ = writeln!(msg, "  default = {}", settings.model.default);
		let _ = writeln!(msg, "  smol    = {}", settings.model.smol);
		let _ = writeln!(msg, "  slow    = {}", settings.model.slow);
		msg.push_str("\nUse /model <name|role> to switch.");
		CommandResult::Message(msg)
	} else {
		let name = ctx.args.trim().to_owned();
		CommandResult::ModelChange(name)
	}
}

/// Handler for `/usage` — stub until token usage tracking is implemented.
pub fn cmd_usage() -> CommandResult {
	CommandResult::Message("Token usage tracking not yet implemented.".to_owned())
}
