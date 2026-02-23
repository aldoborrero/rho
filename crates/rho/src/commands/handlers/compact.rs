//! `/compact` command handler.

use super::super::types::{CommandContext, CommandResult};

/// Handler for `/compact` — returns `Compact` intent with optional
/// instructions.
pub fn cmd_compact(ctx: &CommandContext<'_>) -> CommandResult {
	let instructions = if ctx.args.is_empty() {
		None
	} else {
		Some(ctx.args.to_owned())
	};
	CommandResult::Compact(instructions)
}
