//! Static command registry and input parser.

use super::types::SlashCommand;

/// Static registry of all available slash commands.
pub const COMMANDS: &[SlashCommand] = &[
	SlashCommand {
		name:        "help",
		aliases:     &["h", "?"],
		description: "List all available slash commands",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "exit",
		aliases:     &["quit", "q"],
		description: "Exit the application",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "new",
		aliases:     &[],
		description: "Clear current session",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "model",
		aliases:     &[],
		description: "Show or change the current model",
		args_hint:   Some("[model_name]"),
		subcommands: &[],
	},
	SlashCommand {
		name:        "session",
		aliases:     &[],
		description: "Show session info",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "copy",
		aliases:     &[],
		description: "Copy last AI message to clipboard",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "dump",
		aliases:     &[],
		description: "Copy entire transcript to clipboard",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "usage",
		aliases:     &[],
		description: "Show token usage",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "hotkeys",
		aliases:     &["keys"],
		description: "Show keyboard shortcuts",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "move",
		aliases:     &["cd"],
		description: "Change working directory",
		args_hint:   Some("<path>"),
		subcommands: &[],
	},
	SlashCommand {
		name:        "compact",
		aliases:     &[],
		description: "Compact conversation history to free context space",
		args_hint:   Some("[instructions]"),
		subcommands: &[],
	},
	SlashCommand {
		name:        "plan",
		aliases:     &[],
		description: "Plan mode (not yet implemented)",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "export",
		aliases:     &[],
		description: "Export session (not yet implemented)",
		args_hint:   Some("[path]"),
		subcommands: &[],
	},
	SlashCommand {
		name:        "debug",
		aliases:     &[],
		description: "Dump debug info",
		args_hint:   None,
		subcommands: &[],
	},
	SlashCommand {
		name:        "fork",
		aliases:     &[],
		description: "Fork the current session",
		args_hint:   None,
		subcommands: &[],
	},
];

/// Parse a slash command from raw input.
///
/// Returns `Some((command_name, args))` if the input starts with `/` and
/// matches a registered command (by name or alias). The returned
/// `command_name` is always the canonical name. Returns `None` if no
/// matching command is found.
pub fn parse_command(input: &str) -> Option<(&'static str, &str)> {
	let input = input.trim();
	let stripped = input.strip_prefix('/')?;

	// Split into the command token and the rest (arguments).
	let (token, args) = match stripped.split_once(char::is_whitespace) {
		Some((t, a)) => (t, a.trim()),
		None => (stripped, ""),
	};

	let token_lower = token.to_lowercase();

	// Look up by canonical name or alias.
	for cmd in COMMANDS {
		if cmd.name == token_lower {
			return Some((cmd.name, args));
		}
		for alias in cmd.aliases {
			if *alias == token_lower {
				return Some((cmd.name, args));
			}
		}
	}

	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_help() {
		let result = parse_command("/help");
		assert_eq!(result, Some(("help", "")));
	}

	#[test]
	fn parse_help_alias_h() {
		let result = parse_command("/h");
		assert_eq!(result, Some(("help", "")));
	}

	#[test]
	fn parse_help_alias_question() {
		let result = parse_command("/?");
		assert_eq!(result, Some(("help", "")));
	}

	#[test]
	fn parse_exit() {
		assert_eq!(parse_command("/exit"), Some(("exit", "")));
	}

	#[test]
	fn parse_quit_alias() {
		assert_eq!(parse_command("/quit"), Some(("exit", "")));
	}

	#[test]
	fn parse_q_alias() {
		assert_eq!(parse_command("/q"), Some(("exit", "")));
	}

	#[test]
	fn parse_move_with_args() {
		let result = parse_command("/move /tmp");
		assert_eq!(result, Some(("move", "/tmp")));
	}

	#[test]
	fn parse_cd_alias() {
		let result = parse_command("/cd /tmp");
		assert_eq!(result, Some(("move", "/tmp")));
	}

	#[test]
	fn parse_model_no_args() {
		assert_eq!(parse_command("/model"), Some(("model", "")));
	}

	#[test]
	fn parse_model_with_args() {
		assert_eq!(
			parse_command("/model claude-opus-4-20250514"),
			Some(("model", "claude-opus-4-20250514"))
		);
	}

	#[test]
	fn parse_unknown_command() {
		assert_eq!(parse_command("/foobar"), None);
	}

	#[test]
	fn parse_no_slash() {
		assert_eq!(parse_command("hello world"), None);
	}

	#[test]
	fn parse_empty() {
		assert_eq!(parse_command(""), None);
	}

	#[test]
	fn parse_just_slash() {
		assert_eq!(parse_command("/"), None);
	}

	#[test]
	fn parse_case_insensitive() {
		assert_eq!(parse_command("/HELP"), Some(("help", "")));
		assert_eq!(parse_command("/Exit"), Some(("exit", "")));
	}

	#[test]
	fn parse_with_leading_whitespace() {
		assert_eq!(parse_command("  /help"), Some(("help", "")));
	}

	#[test]
	fn parse_new() {
		assert_eq!(parse_command("/new"), Some(("new", "")));
	}

	#[test]
	fn parse_session() {
		assert_eq!(parse_command("/session"), Some(("session", "")));
	}

	#[test]
	fn parse_copy() {
		assert_eq!(parse_command("/copy"), Some(("copy", "")));
	}

	#[test]
	fn parse_dump() {
		assert_eq!(parse_command("/dump"), Some(("dump", "")));
	}

	#[test]
	fn parse_usage() {
		assert_eq!(parse_command("/usage"), Some(("usage", "")));
	}

	#[test]
	fn parse_hotkeys() {
		assert_eq!(parse_command("/hotkeys"), Some(("hotkeys", "")));
	}

	#[test]
	fn parse_keys_alias() {
		assert_eq!(parse_command("/keys"), Some(("hotkeys", "")));
	}

	#[test]
	fn parse_compact() {
		assert_eq!(parse_command("/compact"), Some(("compact", "")));
	}

	#[test]
	fn parse_plan() {
		assert_eq!(parse_command("/plan"), Some(("plan", "")));
	}

	#[test]
	fn parse_export() {
		assert_eq!(parse_command("/export"), Some(("export", "")));
	}

	#[test]
	fn parse_export_with_path() {
		assert_eq!(parse_command("/export /tmp/out.md"), Some(("export", "/tmp/out.md")));
	}

	#[test]
	fn parse_debug() {
		assert_eq!(parse_command("/debug"), Some(("debug", "")));
	}
}
