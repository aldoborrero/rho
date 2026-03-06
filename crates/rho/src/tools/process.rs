use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Concurrency, OnToolUpdate, Tool, ToolOutput};

/// Default signal for `kill_tree` (SIGTERM).
const DEFAULT_SIGNAL: i32 = 15;

/// Tool that lists or kills processes and their descendants.
pub struct ProcessTool;

#[async_trait]
impl Tool for ProcessTool {
	fn name(&self) -> &'static str {
		"process"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/process.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "action": {
						"type": "string",
						"description": "Action to perform: \"list_descendants\" or \"kill_tree\""
				  },
				  "pid": {
						"type": "integer",
						"description": "Process ID to operate on"
				  },
				  "signal": {
						"type": "integer",
						"description": "Signal number for kill_tree (default: 15 = SIGTERM)"
				  }
			 },
			 "required": ["action", "pid"]
		})
	}

	fn concurrency(&self) -> Concurrency {
		Concurrency::Exclusive
	}

	async fn execute(
		&self,
		input: &Value,
		_cwd: &Path,
		_cancel: &CancellationToken,
		_on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput> {
		let action = input
			.get("action")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: action"))?;

		let pid = input
			.get("pid")
			.and_then(Value::as_i64)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: pid"))? as i32;

		match action {
			"list_descendants" => {
				let result = tokio::task::spawn_blocking(move || rho_tools::ps::list_descendants(pid))
					.await
					.map_err(|e| anyhow::anyhow!("Process list task panicked: {e}"))?;

				if result.is_empty() {
					return Ok(ToolOutput {
						content:  "No descendant processes found.".to_owned(),
						is_error: false,
					});
				}

				let mut output = String::new();
				for child_pid in &result {
					let _ = writeln!(output, "{child_pid}");
				}

				Ok(ToolOutput { content: output.trim_end().to_owned(), is_error: false })
			},
			"kill_tree" => {
				let signal = input
					.get("signal")
					.and_then(Value::as_i64)
					.map_or(DEFAULT_SIGNAL, |v| v as i32);

				let killed = tokio::task::spawn_blocking(move || rho_tools::ps::kill_tree(pid, signal))
					.await
					.map_err(|e| anyhow::anyhow!("Process kill task panicked: {e}"))?;

				Ok(ToolOutput { content: format!("Killed {killed} processes."), is_error: false })
			},
			_ => Ok(ToolOutput {
				content:  format!(
					"Unknown action: {action}. Use \"list_descendants\" or \"kill_tree\"."
				),
				is_error: true,
			}),
		}
	}
}

#[cfg(test)]
mod tests {
	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_process_list_descendants_current() {
		let tool = ProcessTool;
		let ct = CancellationToken::new();
		let pid = std::process::id();
		let result = tool
			.execute(&json!({"action": "list_descendants", "pid": pid}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		// Current process may or may not have children; just verify no error
		assert!(!result.is_error, "Unexpected error: {}", result.content);
	}

	#[tokio::test]
	async fn test_process_unknown_action() {
		let tool = ProcessTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(&json!({"action": "invalid", "pid": 1}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		assert!(result.is_error);
		assert!(
			result.content.contains("Unknown action"),
			"Expected 'Unknown action' in: {}",
			result.content
		);
	}

	#[tokio::test]
	async fn test_process_missing_action() {
		let tool = ProcessTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(&json!({"pid": 1}), Path::new("/"), &ct, None)
			.await;
		assert!(result.is_err(), "Expected error for missing action parameter");
	}
}
