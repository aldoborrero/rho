//! Command dispatch — routes `CommandContext` to the appropriate handler.

use super::{
	handlers,
	types::{CommandContext, CommandResult},
};
use crate::tools::registry::ToolRegistry;

/// Execute a slash command given a fully-populated context.
#[allow(
	clippy::future_not_send,
	reason = "CommandContext borrows SessionManager which contains dyn SessionStorage (not Sync)"
)]
pub async fn execute_command(ctx: &CommandContext<'_>) -> anyhow::Result<CommandResult> {
	match ctx.name {
		"help" => Ok(handlers::help::cmd_help()),
		"exit" => Ok(CommandResult::Exit),
		"new" => Ok(CommandResult::NewSession),
		"model" => Ok(handlers::model::cmd_model(ctx)),
		"session" => Ok(handlers::session::cmd_session(ctx)),
		"copy" => handlers::clipboard::cmd_copy(ctx).await,
		"dump" => handlers::clipboard::cmd_dump(ctx).await,
		"usage" => Ok(handlers::model::cmd_usage()),
		"hotkeys" => Ok(handlers::help::cmd_hotkeys()),
		"move" => handlers::navigation::cmd_move(ctx),
		"compact" => Ok(handlers::compact::cmd_compact(ctx)),
		"plan" => Ok(handlers::plan::cmd_plan()),
		"export" => Ok(handlers::session::cmd_export()),
		"config" => Ok(handlers::config_cmd::cmd_config(ctx)),
		"debug" => Ok(handlers::session::cmd_debug(ctx)),
		"fork" => Ok(handlers::session::cmd_fork()),
		_ => Ok(CommandResult::Message(format!(
			"Unknown command: /{}. Type /help for available commands.",
			ctx.name
		))),
	}
}

/// Execute a `!` shell command directly, returning structured output.
pub async fn execute_bang(
	command: &str,
	tools: &ToolRegistry,
) -> anyhow::Result<rho_agent::tools::ToolOutput> {
	let cwd = std::env::current_dir()?;
	let ct = tokio_util::sync::CancellationToken::new();
	tools
		.execute("bash", serde_json::json!({"command": command}), &cwd, &ct)
		.await
}

/// Result of a streaming bang command execution.
pub struct BangResult {
	pub is_error:  bool,
	pub exit_code: Option<i32>,
	pub cancelled: bool,
}

/// Execute a `!` shell command with streaming output callback.
///
/// Calls `rho_tools::shell::execute_shell` directly (bypassing `ToolRegistry`)
/// to enable streaming chunks via `on_chunk`.
pub async fn execute_bang_streaming<F>(
	command: &str,
	_tools: &ToolRegistry,
	on_chunk: F,
) -> anyhow::Result<BangResult>
where
	F: Fn(String) + Send + Sync + 'static,
{
	let cwd = std::env::current_dir()?;
	let options = rho_tools::shell::ShellExecuteOptions {
		command:       command.to_owned(),
		cwd:           Some(cwd.to_string_lossy().into_owned()),
		env:           None,
		session_env:   None,
		snapshot_path: None,
	};
	let on_chunk_box: Box<dyn Fn(String) + Send + Sync> = Box::new(on_chunk);
	let ct = rho_tools::cancel::CancelToken::new(Some(30_000)); // 30s timeout for dispatch
	let result = rho_tools::shell::execute_shell(options, Some(on_chunk_box), ct).await?;

	let is_error = result.exit_code.is_none_or(|c| c != 0);
	Ok(BangResult {
		is_error,
		exit_code: result.exit_code,
		cancelled: result.cancelled,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{session::SessionManager, settings::Settings};

	fn test_settings() -> Settings {
		Settings::default()
	}

	#[tokio::test]
	async fn execute_help_returns_message() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "help",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("/help"));
				assert!(text.contains("/exit"));
				assert!(text.contains("/move"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_exit_returns_exit() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "exit",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		assert!(matches!(result, CommandResult::Exit));
	}

	#[tokio::test]
	async fn execute_new_returns_new_session() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "new",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		assert!(matches!(result, CommandResult::NewSession));
	}

	#[tokio::test]
	async fn execute_model_no_args_shows_current() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "model",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "claude-sonnet-4-5-20250929",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("claude-sonnet-4-5-20250929"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_model_with_args_returns_model_change() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "model",
			args:    "claude-opus-4-20250514",
			session: &session,
			settings: &settings,
			model:   "claude-sonnet-4-5-20250929",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::ModelChange(name) => {
				assert_eq!(name, "claude-opus-4-20250514");
			},
			_ => panic!("Expected CommandResult::ModelChange"),
		}
	}

	#[tokio::test]
	async fn execute_session_shows_info() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "session",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("Session ID:"));
				assert!(text.contains("Messages:"));
				assert!(text.contains("Entries:"));
				assert!(text.contains("Working directory:"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_copy_no_messages() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "copy",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("No assistant messages"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_dump_no_messages() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "dump",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("No messages"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_usage_placeholder() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "usage",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("not yet implemented"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_hotkeys_returns_shortcuts() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "hotkeys",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("Enter"));
				assert!(text.contains("Ctrl+C"));
				assert!(text.contains("Ctrl+D"));
				assert!(text.contains("Ctrl+L"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_move_no_args() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "move",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("Usage:"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_move_valid_dir() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "move",
			args:    "/tmp",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		assert!(matches!(result, CommandResult::ChangeDir(_)));
	}

	#[tokio::test]
	async fn execute_move_nonexistent_dir() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "move",
			args:    "/nonexistent_path_12345",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("Not a directory"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_compact_returns_compact_variant() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "compact",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		assert!(matches!(result, CommandResult::Compact(None)));
	}

	#[tokio::test]
	async fn execute_compact_with_instructions() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "compact",
			args:    "focus on errors",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Compact(Some(instructions)) => {
				assert_eq!(instructions, "focus on errors");
			},
			_ => panic!("Expected CommandResult::Compact(Some(_))"),
		}
	}

	#[tokio::test]
	async fn execute_plan_placeholder() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "plan",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("not yet implemented"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_export_placeholder() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "export",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("not yet implemented"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_debug_shows_info() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "debug",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("Session ID:"));
				assert!(text.contains("Model: test-model"));
				assert!(text.contains("Entries:"));
				assert!(text.contains("Leaf ID:"));
				assert!(text.contains("Terminal size:"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}

	#[tokio::test]
	async fn execute_fork_returns_fork_variant() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "fork",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		assert!(matches!(result, CommandResult::Fork));
	}

	#[tokio::test]
	async fn execute_unknown_returns_message() {
		let session = SessionManager::in_memory();
		let settings = test_settings();
		let tools = ToolRegistry::new();
		let ctx = CommandContext {
			name:    "nonexistent",
			args:    "",
			session: &session,
			settings: &settings,
			model:   "test-model",
			tools:   &tools,
		};
		let result = execute_command(&ctx).await.unwrap();
		match result {
			CommandResult::Message(text) => {
				assert!(text.contains("Unknown command"));
				assert!(text.contains("nonexistent"));
			},
			_ => panic!("Expected CommandResult::Message"),
		}
	}
}
