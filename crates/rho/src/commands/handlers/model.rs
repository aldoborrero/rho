//! `/model` and `/usage` command handlers.

use super::super::types::{CommandContext, CommandResult};

/// Handler for `/model` — show or change the active model.
pub fn cmd_model(ctx: &CommandContext<'_>) -> CommandResult {
	if ctx.args.is_empty() {
		CommandResult::Message(format!("Current model: {}", ctx.model))
	} else {
		CommandResult::Message(format!("Model switching not yet implemented. Current: {}", ctx.model))
	}
}

/// Handler for `/usage` — stub until token usage tracking is implemented.
pub fn cmd_usage() -> CommandResult {
	CommandResult::Message("Token usage tracking not yet implemented.".to_owned())
}
