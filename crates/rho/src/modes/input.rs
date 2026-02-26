//! Input classification for the interactive event loop.
//!
//! [`route_input`] is a pure classifier — it reads no state and performs no
//! side effects. The event loop builds [`CommandContext`] and dispatches
//! after classification.

/// What the input router decided to do with a line of user input.
pub enum InputAction<'a> {
	/// A recognized slash command.
	SlashCommand { name: &'static str, args: &'a str },
	/// A `/`-prefixed input that didn't match any registered command.
	UnknownCommand(&'a str),
	/// A `!`-prefixed shell command (`!!` excludes from LLM context).
	BangCommand { cmd: &'a str, exclude_from_context: bool },
	/// Normal message to send to the agent.
	UserMessage(&'a str),
	/// Empty input, ignore.
	Empty,
}

/// Classify raw user input into an [`InputAction`].
///
/// This is a pure function: it reads no session state and performs no I/O.
pub fn route_input(text: &str) -> InputAction<'_> {
	let text = text.trim();
	if text.is_empty() {
		return InputAction::Empty;
	}

	if text.starts_with('/') {
		return match crate::commands::parse_command(text) {
			Some((name, args)) => InputAction::SlashCommand { name, args },
			None => InputAction::UnknownCommand(text.split_whitespace().next().unwrap_or(text)),
		};
	}

	if text.starts_with("!!") {
		return InputAction::BangCommand {
			cmd:                  &text[2..],
			exclude_from_context: true,
		};
	}
	if text.starts_with('!') {
		return InputAction::BangCommand {
			cmd:                  &text[1..],
			exclude_from_context: false,
		};
	}

	InputAction::UserMessage(text)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty_input() {
		assert!(matches!(route_input(""), InputAction::Empty));
	}

	#[test]
	fn whitespace_only() {
		assert!(matches!(route_input("   "), InputAction::Empty));
	}

	#[test]
	fn slash_command_help() {
		match route_input("/help") {
			InputAction::SlashCommand { name, args } => {
				assert_eq!(name, "help");
				assert_eq!(args, "");
			},
			_ => panic!("Expected SlashCommand"),
		}
	}

	#[test]
	fn slash_command_with_args() {
		match route_input("/model claude-opus-4-20250514") {
			InputAction::SlashCommand { name, args } => {
				assert_eq!(name, "model");
				assert_eq!(args, "claude-opus-4-20250514");
			},
			_ => panic!("Expected SlashCommand"),
		}
	}

	#[test]
	fn unknown_slash_command() {
		match route_input("/foobar") {
			InputAction::UnknownCommand(cmd) => {
				assert_eq!(cmd, "/foobar");
			},
			_ => panic!("Expected UnknownCommand"),
		}
	}

	#[test]
	fn bang_command() {
		match route_input("!ls -la") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "ls -la");
				assert!(!exclude_from_context);
			},
			_ => panic!("Expected BangCommand"),
		}
	}

	#[test]
	fn double_bang_is_bang_excluded() {
		match route_input("!!ls -la") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "ls -la");
				assert!(exclude_from_context);
			},
			_ => panic!("Expected BangCommand with exclude_from_context"),
		}
	}

	#[test]
	fn single_bang_not_excluded() {
		match route_input("!pwd") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "pwd");
				assert!(!exclude_from_context);
			},
			_ => panic!("Expected BangCommand"),
		}
	}

	#[test]
	fn normal_message() {
		match route_input("hello world") {
			InputAction::UserMessage(text) => {
				assert_eq!(text, "hello world");
			},
			_ => panic!("Expected UserMessage"),
		}
	}

	#[test]
	fn slash_alias() {
		match route_input("/q") {
			InputAction::SlashCommand { name, args } => {
				assert_eq!(name, "exit");
				assert_eq!(args, "");
			},
			_ => panic!("Expected SlashCommand"),
		}
	}

	#[test]
	fn move_with_path() {
		match route_input("/cd /tmp") {
			InputAction::SlashCommand { name, args } => {
				assert_eq!(name, "move");
				assert_eq!(args, "/tmp");
			},
			_ => panic!("Expected SlashCommand"),
		}
	}

	#[test]
	fn bang_single_char() {
		match route_input("!x") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "x");
				assert!(!exclude_from_context);
			},
			_ => panic!("Expected BangCommand"),
		}
	}

	#[test]
	fn leading_whitespace_trimmed() {
		match route_input("  /help  ") {
			InputAction::SlashCommand { name, .. } => {
				assert_eq!(name, "help");
			},
			_ => panic!("Expected SlashCommand"),
		}
	}
}
