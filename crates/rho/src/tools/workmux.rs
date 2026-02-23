use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Tool, ToolOutput};

/// Tool that manages terminal multiplexer windows and agents.
pub struct WorkmuxTool;

#[async_trait]
impl Tool for WorkmuxTool {
	fn name(&self) -> &'static str {
		"workmux"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/workmux.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "action": {
						"type": "string",
						"description": "Action: \"detect\", \"list_agents\", \"create_window\", \"send_keys\", or \"capture_pane\""
				  },
				  "prefix": {
						"type": "string",
						"description": "Window name prefix (for create_window)"
				  },
				  "name": {
						"type": "string",
						"description": "Window name (for create_window)"
				  },
				  "cwd": {
						"type": "string",
						"description": "Working directory (for create_window)"
				  },
				  "pane_id": {
						"type": "string",
						"description": "Target pane identifier (for send_keys, capture_pane)"
				  },
				  "keys": {
						"type": "string",
						"description": "Keys/command to send (for send_keys)"
				  },
				  "lines": {
						"type": "integer",
						"description": "Number of lines to capture (for capture_pane, default: 50)"
				  }
			 },
			 "required": ["action"]
		})
	}

	async fn execute(&self, input: Value, _cwd: &Path) -> anyhow::Result<ToolOutput> {
		let action = input
			.get("action")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: action"))?;

		match action {
			"detect" => {
				let result =
					tokio::task::spawn_blocking(rho_tools::workmux::workmux_detect_environment)
						.await
						.map_err(|e| anyhow::anyhow!("Workmux detect task panicked: {e}"))?;

				match result {
					Ok(env) => {
						let mut output = String::new();
						let _ = writeln!(output, "Backend: {:?}", env.backend);
						let _ = writeln!(output, "Running: {}", env.is_running);
						if let Some(pane_id) = &env.pane_id {
							let _ = writeln!(output, "Pane ID: {pane_id}");
						} else {
							let _ = writeln!(output, "Pane ID: none");
						}
						Ok(ToolOutput { content: output.trim_end().to_owned(), is_error: false })
					},
					Err(e) => {
						Ok(ToolOutput { content: format!("Workmux detect error: {e}"), is_error: true })
					},
				}
			},
			"list_agents" => {
				let result = tokio::task::spawn_blocking(rho_tools::workmux::workmux_list_agents)
					.await
					.map_err(|e| anyhow::anyhow!("Workmux list agents task panicked: {e}"))?;

				match result {
					Ok(agents) => {
						if agents.is_empty() {
							return Ok(ToolOutput {
								content:  "No agents found.".to_owned(),
								is_error: false,
							});
						}
						let mut output = String::new();
						for agent in &agents {
							let status = agent
								.status
								.as_ref()
								.map_or_else(|| "unknown".to_owned(), |s| format!("{s:?}"));
							let title = agent.title.as_deref().unwrap_or("untitled");
							let _ = writeln!(
								output,
								"Pane: {} | Status: {} | Title: {} | Dir: {}",
								agent.pane_id, status, title, agent.workdir
							);
						}
						Ok(ToolOutput { content: output.trim_end().to_owned(), is_error: false })
					},
					Err(e) => Ok(ToolOutput {
						content:  format!("Workmux list agents error: {e}"),
						is_error: true,
					}),
				}
			},
			"create_window" => {
				let prefix = input
					.get("prefix")
					.and_then(Value::as_str)
					.ok_or_else(|| {
						anyhow::anyhow!("Missing required parameter for create_window: prefix")
					})?
					.to_owned();

				let name = input
					.get("name")
					.and_then(Value::as_str)
					.ok_or_else(|| {
						anyhow::anyhow!("Missing required parameter for create_window: name")
					})?
					.to_owned();

				let window_cwd = input
					.get("cwd")
					.and_then(Value::as_str)
					.ok_or_else(|| anyhow::anyhow!("Missing required parameter for create_window: cwd"))?
					.to_owned();

				let params = rho_tools::workmux::WorkmuxCreateWindowParams {
					prefix,
					name,
					cwd: window_cwd,
					after_window: None,
				};

				let result = tokio::task::spawn_blocking(move || {
					rho_tools::workmux::workmux_create_window(params)
				})
				.await
				.map_err(|e| anyhow::anyhow!("Workmux create window task panicked: {e}"))?;

				match result {
					Ok(pane_id) => Ok(ToolOutput {
						content:  format!("Window created. Pane ID: {pane_id}"),
						is_error: false,
					}),
					Err(e) => Ok(ToolOutput {
						content:  format!("Workmux create window error: {e}"),
						is_error: true,
					}),
				}
			},
			"send_keys" => {
				let pane_id = input
					.get("pane_id")
					.and_then(Value::as_str)
					.ok_or_else(|| anyhow::anyhow!("Missing required parameter for send_keys: pane_id"))?
					.to_owned();

				let keys = input
					.get("keys")
					.and_then(Value::as_str)
					.ok_or_else(|| anyhow::anyhow!("Missing required parameter for send_keys: keys"))?
					.to_owned();

				let result = tokio::task::spawn_blocking(move || {
					rho_tools::workmux::workmux_send_keys(&pane_id, &keys)
				})
				.await
				.map_err(|e| anyhow::anyhow!("Workmux send keys task panicked: {e}"))?;

				match result {
					Ok(()) => {
						Ok(ToolOutput { content: "Keys sent successfully.".to_owned(), is_error: false })
					},
					Err(e) => Ok(ToolOutput {
						content:  format!("Workmux send keys error: {e}"),
						is_error: true,
					}),
				}
			},
			"capture_pane" => {
				let pane_id = input
					.get("pane_id")
					.and_then(Value::as_str)
					.ok_or_else(|| {
						anyhow::anyhow!("Missing required parameter for capture_pane: pane_id")
					})?
					.to_owned();

				let lines = input.get("lines").and_then(Value::as_u64).map(|v| v as u32);

				let result = tokio::task::spawn_blocking(move || {
					rho_tools::workmux::workmux_capture_pane(&pane_id, lines)
				})
				.await
				.map_err(|e| anyhow::anyhow!("Workmux capture pane task panicked: {e}"))?;

				match result {
					Ok(Some(content)) => Ok(ToolOutput { content, is_error: false }),
					Ok(None) => Ok(ToolOutput {
						content:  "No content captured (multiplexer may not be running).".to_owned(),
						is_error: false,
					}),
					Err(e) => Ok(ToolOutput {
						content:  format!("Workmux capture pane error: {e}"),
						is_error: true,
					}),
				}
			},
			_ => Ok(ToolOutput {
				content:  format!(
					"Unknown action: {action}. Use \"detect\", \"list_agents\", \"create_window\", \
					 \"send_keys\", or \"capture_pane\"."
				),
				is_error: true,
			}),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_workmux_detect() {
		let tool = WorkmuxTool;
		let result = tool
			.execute(json!({"action": "detect"}), Path::new("/"))
			.await
			.unwrap();
		// Detection should succeed even without a running multiplexer
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(result.content.contains("Backend:"), "Expected 'Backend:' in: {}", result.content);
	}

	#[tokio::test]
	async fn test_workmux_unknown_action() {
		let tool = WorkmuxTool;
		let result = tool
			.execute(json!({"action": "invalid"}), Path::new("/"))
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
	async fn test_workmux_missing_action() {
		let tool = WorkmuxTool;
		let result = tool.execute(json!({}), Path::new("/")).await;
		assert!(result.is_err(), "Expected error for missing action parameter");
	}
}
