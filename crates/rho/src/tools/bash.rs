use std::{
	path::Path,
	sync::{Arc, Mutex},
};

use async_trait::async_trait;
use rho_tools::shell::{ShellExecuteOptions, execute_shell};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Concurrency, OnToolUpdate, Tool, ToolOutput};

/// Maximum output size in bytes (100 KB).
const MAX_OUTPUT_BYTES: usize = 100 * 1024;

/// Default timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// A buffer that keeps only the last `max_bytes` of appended content.
///
/// Tracks total bytes seen for truncation reporting.
struct TailBuffer {
	buf:         String,
	max_bytes:   usize,
	total_bytes: usize,
	truncated:   bool,
}

impl TailBuffer {
	const fn new(max_bytes: usize) -> Self {
		Self { buf: String::new(), max_bytes, total_bytes: 0, truncated: false }
	}

	fn append(&mut self, chunk: &str) {
		self.total_bytes += chunk.len();
		self.buf.push_str(chunk);
		if self.buf.len() > self.max_bytes {
			self.truncated = true;
			let excess = self.buf.len() - self.max_bytes;
			// Round up to the next UTF-8 character boundary to avoid
			// panicking on multi-byte characters at the drain point.
			let drain_to = self.buf.ceil_char_boundary(excess);
			self.buf.drain(..drain_to);
		}
	}

	fn text(&self) -> &str {
		&self.buf
	}
}

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

	fn concurrency(&self) -> Concurrency {
		Concurrency::Exclusive
	}

	async fn execute(
		&self,
		input: &Value,
		cwd: &Path,
		cancel: &CancellationToken,
		on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput> {
		let command = input
			.get("command")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;

		let timeout_secs = input
			.get("timeout")
			.and_then(Value::as_u64)
			.unwrap_or(DEFAULT_TIMEOUT_SECS);

		// Collect streaming output into a bounded tail buffer.
		let output = Arc::new(Mutex::new(TailBuffer::new(MAX_OUTPUT_BYTES)));
		let output_clone = output.clone();
		let on_update_clone = on_update.cloned();
		let on_chunk: Box<dyn Fn(String) + Send + Sync> = Box::new(move |chunk: String| {
			output_clone
				.lock()
				.unwrap_or_else(|e| e.into_inner())
				.append(&chunk);
			if let Some(ref cb) = on_update_clone {
				cb(&chunk);
			}
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

				let buf = output.lock().unwrap_or_else(|e| e.into_inner());
				let text = if buf.truncated {
					format!(
						"[Output truncated: showing last {}KB of {}KB total]\n{}",
						MAX_OUTPUT_BYTES / 1024,
						buf.total_bytes / 1024,
						buf.text()
					)
				} else {
					buf.text().to_owned()
				};
				drop(buf);

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
			.execute(&json!({"command": "echo hello"}), Path::new("."), &ct, None)
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
			.execute(&json!({"command": "false"}), Path::new("."), &ct, None)
			.await
			.unwrap();
		assert!(result.is_error);
	}

	#[tokio::test]
	async fn test_bash_missing_command() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		let result = tool.execute(&json!({}), Path::new("."), &ct, None).await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_bash_cwd() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(&json!({"command": "pwd"}), Path::new("/tmp"), &ct, None)
			.await
			.unwrap();
		assert_eq!(result.content.trim(), "/tmp");
		assert!(!result.is_error);
	}

	#[test]
	fn test_tail_buffer_small_input() {
		let mut buf = TailBuffer::new(100);
		buf.append("hello ");
		buf.append("world");
		assert_eq!(buf.text(), "hello world");
		assert_eq!(buf.total_bytes, 11);
		assert!(!buf.truncated);
	}

	#[test]
	fn test_tail_buffer_truncates_to_tail() {
		let mut buf = TailBuffer::new(10);
		buf.append("aaaaaaaaaa"); // 10 bytes, exactly at limit
		assert_eq!(buf.text(), "aaaaaaaaaa");
		assert!(!buf.truncated);

		buf.append("bbbbb"); // 5 more bytes, now 15 total, buffer keeps last 10
		assert_eq!(buf.total_bytes, 15);
		assert!(buf.truncated);
		assert_eq!(buf.text().len(), 10);
		assert_eq!(buf.text(), "aaaaabbbbb"); // tail of the content
	}

	#[test]
	fn test_tail_buffer_large_single_chunk() {
		let mut buf = TailBuffer::new(10);
		buf.append("abcdefghijklmnop"); // 16 bytes in one chunk
		assert_eq!(buf.total_bytes, 16);
		assert!(buf.truncated);
		assert_eq!(buf.text(), "ghijklmnop"); // last 10 bytes
	}

	#[test]
	fn test_tail_buffer_multibyte_utf8() {
		// Buffer of 5 bytes. Fill with 4 bytes of ASCII, then append a 3-byte
		// UTF-8 char (e.g. 'あ' = 3 bytes). Total = 7, excess = 2, but byte
		// offset 2 might fall inside a multi-byte char at the front.
		let mut buf = TailBuffer::new(5);
		buf.append("abcd"); // 4 bytes
		buf.append("あ"); // 3 bytes (UTF-8: 0xE3 0x81 0x82) -> total 7, excess 2
		assert!(buf.truncated);
		// drain_to rounds up to char boundary (3 for 'a','b','c' are 1-byte each,
		// so draining 2 bytes is safe here). Result keeps last 5 bytes.
		assert!(buf.text().is_char_boundary(0)); // must be valid UTF-8
		assert_eq!(buf.total_bytes, 7);

		// Harder case: buffer starts with multi-byte chars
		let mut buf2 = TailBuffer::new(6);
		buf2.append("ああ"); // 6 bytes (2 * 3-byte chars), exactly at limit
		assert!(!buf2.truncated);
		buf2.append("x"); // 7 bytes total, excess = 1, but byte 1 is mid-char
		assert!(buf2.truncated);
		// Should round up to drain 3 bytes (full first 'あ'), keeping "あx"
		assert_eq!(buf2.text(), "あx");
		assert_eq!(buf2.total_bytes, 7);
	}

	#[tokio::test]
	async fn test_bash_large_output_truncated() {
		let tool = BashTool;
		let ct = CancellationToken::new();
		// Generate output larger than MAX_OUTPUT_BYTES (100KB).
		let result = tool
			.execute(&json!({"command": "head -c 204800 /dev/urandom | base64"}), Path::new("."), &ct, None)
			.await
			.unwrap();
		// Output should contain the truncation notice.
		assert!(
			result.content.starts_with("[Output truncated:"),
			"Expected truncation notice, got: {}",
			&result.content[..80.min(result.content.len())]
		);
		assert!(!result.is_error);
	}
}
