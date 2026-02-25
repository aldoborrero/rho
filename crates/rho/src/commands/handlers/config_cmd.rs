//! `/config` command handler — view and modify settings.

use std::fmt::Write;

use super::super::types::{CommandContext, CommandResult};

/// Handler for `/config [subcommand]`.
pub fn cmd_config(ctx: &CommandContext<'_>) -> CommandResult {
	let (subcommand, rest) = split_first_word(ctx.args);
	match subcommand {
		"list" | "" => cmd_config_list(),
		"get" => cmd_config_get(rest),
		"set" => cmd_config_set(rest),
		"reset" => cmd_config_reset(rest),
		"path" => cmd_config_path(),
		_ => CommandResult::Message(format!(
			"Unknown subcommand: {subcommand}. Use: list, get, set, reset, path"
		)),
	}
}

fn cmd_config_list() -> CommandResult {
	let items = crate::settings::list_all();
	if items.is_empty() {
		return CommandResult::Message("No settings configured (using defaults).".to_owned());
	}
	let mut out = String::from("Current settings:\n\n");
	for (key, value) in &items {
		let _ = writeln!(out, "  {key} = {value}");
	}
	CommandResult::Message(out)
}

fn cmd_config_get(args: &str) -> CommandResult {
	let key = args.trim();
	if key.is_empty() {
		return CommandResult::Message("Usage: /config get <key>\n\nExample: /config get agent.max_tokens".to_owned());
	}
	match crate::settings::get(key) {
		Some(value) => CommandResult::Message(format!("{key} = {value}")),
		None => CommandResult::Message(format!("Key not found: {key}")),
	}
}

fn cmd_config_set(args: &str) -> CommandResult {
	let (key, value) = split_first_word(args);
	if key.is_empty() || value.is_empty() {
		return CommandResult::Message(
			"Usage: /config set <key> <value>\n\nExample: /config set agent.max_tokens 16384"
				.to_owned(),
		);
	}
	match crate::settings::set(key, value) {
		Ok(()) => {
			CommandResult::SettingsChanged
		},
		Err(e) => CommandResult::Message(format!("Failed to set {key}: {e}")),
	}
}

fn cmd_config_reset(args: &str) -> CommandResult {
	let key = args.trim();
	if key.is_empty() {
		return CommandResult::Message("Usage: /config reset <key>".to_owned());
	}
	match crate::settings::reset(key) {
		Ok(()) => {
			CommandResult::SettingsChanged
		},
		Err(e) => CommandResult::Message(format!("Failed to reset {key}: {e}")),
	}
}

fn cmd_config_path() -> CommandResult {
	match crate::settings::global_config_path() {
		Some(path) => CommandResult::Message(format!("Global config: {}", path.display())),
		None => CommandResult::Message("Cannot determine home directory.".to_owned()),
	}
}

/// Split a string into the first whitespace-delimited word and the rest.
fn split_first_word(s: &str) -> (&str, &str) {
	let s = s.trim();
	match s.split_once(char::is_whitespace) {
		Some((first, rest)) => (first, rest.trim()),
		None => (s, ""),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn split_first_word_basic() {
		assert_eq!(split_first_word("get agent.max_tokens"), ("get", "agent.max_tokens"));
	}

	#[test]
	fn split_first_word_single() {
		assert_eq!(split_first_word("list"), ("list", ""));
	}

	#[test]
	fn split_first_word_empty() {
		assert_eq!(split_first_word(""), ("", ""));
	}

	#[test]
	fn split_first_word_with_extra_spaces() {
		assert_eq!(split_first_word("  set   agent.max_tokens  16384  "), ("set", "agent.max_tokens  16384"));
	}
}
