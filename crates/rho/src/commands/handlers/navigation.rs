//! `/move` (alias `/cd`) command handler.

use std::path::PathBuf;

use super::super::types::{CommandContext, CommandResult};

/// Handler for `/move` — change the working directory.
pub fn cmd_move(ctx: &CommandContext<'_>) -> anyhow::Result<CommandResult> {
	if ctx.args.is_empty() {
		return Ok(CommandResult::Message("Usage: /move <path>".to_owned()));
	}

	let path = PathBuf::from(ctx.args);
	let resolved = if path.is_absolute() {
		path
	} else {
		std::env::current_dir()?.join(path)
	};

	if !resolved.is_dir() {
		return Ok(CommandResult::Message(format!("Not a directory: {}", resolved.display())));
	}

	// Canonicalize to resolve symlinks and `..` components.
	let canonical = resolved.canonicalize()?;
	Ok(CommandResult::ChangeDir(canonical.display().to_string()))
}
