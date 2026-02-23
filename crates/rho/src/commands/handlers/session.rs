//! `/session`, `/debug`, `/export`, `/fork` command handlers.

use std::fmt::Write as _;

use super::super::types::{CommandContext, CommandResult};

/// Handler for `/session` — display session metadata.
pub fn cmd_session(ctx: &CommandContext<'_>) -> CommandResult {
	let cwd = std::env::current_dir().map_or_else(|_| ".".to_owned(), |p| p.display().to_string());
	let mut info = format!("Session ID: {}\n", ctx.session.session_id());
	if let Some(title) = ctx.session.header().title.as_deref() {
		let _ = writeln!(info, "Title: {title}");
	}
	let _ = writeln!(info, "Messages: {}", ctx.session.messages().len());
	let _ = writeln!(info, "Entries: {}", ctx.session.entries().len());
	let _ = write!(info, "Working directory: {cwd}");
	CommandResult::Message(info)
}

/// Handler for `/debug` — dump internal debug information.
pub fn cmd_debug(ctx: &CommandContext<'_>) -> CommandResult {
	let cwd = std::env::current_dir().map_or_else(|_| ".".to_owned(), |p| p.display().to_string());
	let term_size = crossterm::terminal::size()
		.map_or_else(|_| "unknown".to_owned(), |(c, r)| format!("{c}x{r}"));
	let leaf = ctx.session.leaf_id().unwrap_or("(none)");
	let info = format!(
		"Debug info:\n  Session ID: {}\n  Messages: {}\n  Entries: {}\n  Leaf ID: {leaf}\n  Model: \
		 {}\n  Working directory: {cwd}\n  Terminal size: {term_size}",
		ctx.session.session_id(),
		ctx.session.messages().len(),
		ctx.session.entries().len(),
		ctx.model,
	);
	CommandResult::Message(info)
}

/// Handler for `/export` — stub until export is implemented.
pub fn cmd_export() -> CommandResult {
	CommandResult::Message("Export not yet implemented.".to_owned())
}

/// Handler for `/fork` — returns `Fork` intent for the event loop.
pub const fn cmd_fork() -> CommandResult {
	CommandResult::Fork
}
