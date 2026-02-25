use std::{
	path::Path,
	sync::{Arc, Mutex},
};

use async_trait::async_trait;
use rho_tools::shell::{ShellExecuteOptions, execute_shell};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Tool, ToolOutput};

/// Maximum output size in bytes (100 KB).
const MAX_OUTPUT_BYTES: usize = 100 * 1024;

/// Default timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Tool that executes shell commands via the rho-tools brush-core shell.
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
	fn name(&self) -> &'static str {
		"bash"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/bash.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "command": {
						"type": "string",
						"description": "The bash command to execute"
				  },
				  "timeout": {
						"type": "integer",
						"description": "Timeout in seconds (default: 300)"
				  }
			 },
			 "required": ["command"]
		})
	}

	async fn execute(&self, input: Value, cwd: &Path, cancel: &CancellationToken) -> anyhow::Result<ToolOutput> {
		let command = input
			.get("command")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;

		let timeout_secs = input
			.get("timeout")
			.and_then(Value::as_u64)
			.unwrap_or(DEFAULT_TIMEOUT_SECS);

		// Collect streaming output into a shared buffer.
		let output = Arc::new(Mutex::new(String::new()));
		let output_clone = output.clone();
		let on_chunk: Box<dyn Fn(String) + Send + Sync> = Box::new(move |chunk: String| {
			output_clone
				.lock()
				.expect("output mutex poisoned")
				.push_str(&chunk);
		});

		let options = ShellExecuteOptions {
			command:       command.to_owned(),
			cwd:           Some(cwd.to_string_lossy().into_owned()),
			env:           None,
			session_env:   None,
			snapshot_path: None,
		};

		let mut ct = rho_tools::cancel::CancelToken::new(Some(
			u32::try_from(timeout_secs * 1000).unwrap_or(u32::MAX),
		));
		let internal_abort = ct.emplace_abort_token();

		// Bridge: external CancellationToken → internal CancelToken.
		let external = cancel.clone();
		let bridge = tokio::spawn(async move {
			external.cancelled().await;
			internal_abort.abort(rho_tools::cancel::AbortReason::Signal);
		});

		let result = execute_shell(options, Some(on_chunk), ct).await;
		bridge.abort();

		match result {
			Ok(result) => {
				if result.timed_out {
					return Ok(ToolOutput {
						content:  format!("Command timed out after {timeout_secs}s"),
						is_error: true,
					});
				}
				if result.cancelled {
					return Ok(ToolOutput {
						content:  "Command was cancelled.".to_owned(),
						is_error: true,
					});
				}

				let mut text = output.lock().expect("output mutex poisoned").clone();
				if text.len() > MAX_OUTPUT_BYTES {
					text.truncate(MAX_OUTPUT_BYTES);
					text.push_str("\n... (output truncated)");
				}

				let is_error = result.exit_code.is_none_or(|c| c != 0);
				Ok(ToolOutput { content: text, is_error })
			},
			Err(e) => {
				Ok(ToolOutput { content: format!("Failed to execute command: {e}"), is_error: true })
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_bash_echo() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"command": "echo hello"}), Path::new("."), &ct)
			.await
			.unwrap();
		assert_eq!(result.content.trim(), "hello");
		assert!(!result.is_error);
	}

	#[tokio::test]
	async fn test_bash_error() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"command": "false"}), Path::new("."), &ct)
			.await
			.unwrap();
		assert!(result.is_error);
	}

	#[tokio::test]
	async fn test_bash_missing_command() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		let result = tool.execute(json!({}), Path::new("."), &ct).await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_bash_cwd() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"command": "pwd"}), Path::new("/tmp"), &ct)
			.await
			.unwrap();
		assert_eq!(result.content.trim(), "/tmp");
		assert!(!result.is_error);
	}
}
