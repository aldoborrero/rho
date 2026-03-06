use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::types::Message;

/// Concurrency mode for tool scheduling when multiple calls arrive in one turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Concurrency {
	/// Can run alongside other shared tools.
	#[default]
	Shared,
	/// Runs alone — all prior tools must finish before this starts,
	/// and this must finish before the next tool starts.
	Exclusive,
}

/// Output from executing a tool.
pub struct ToolOutput {
	pub content:  String,
	pub is_error: bool,
}

/// Callback type for streaming incremental tool output to the UI.
///
/// Takes a `&str` slice (not `String`) to avoid allocation per chunk.
/// Synchronous (`Fn`, not `async`) — the agent loop bridges to the async
/// event channel via `try_send()`. Uses `Arc` so the callback can be
/// cheaply cloned into inner closures (e.g. bash `on_chunk`).
pub type OnToolUpdate = Arc<dyn Fn(&str) + Send + Sync>;

/// Synchronous callback that drains queued messages from the caller.
/// Returns an empty `Vec` when no messages are pending. Polled at tool
/// execution boundaries (steering) and after the inner loop exhausts
/// (follow-up). Uses `Fn` (not async) because the backing store is a
/// `Mutex<VecDeque>` — no await needed.
pub type MessageFetcher = Arc<dyn Fn() -> Vec<Message> + Send + Sync>;

/// Trait for tools that the AI can invoke.
#[async_trait]
pub trait Tool: Send + Sync {
	/// The tool name (used in API calls).
	fn name(&self) -> &str;

	/// Human-readable description of the tool.
	fn description(&self) -> &str;

	/// JSON Schema for the tool's input.
	fn input_schema(&self) -> serde_json::Value;

	/// Concurrency mode. Default: [`Concurrency::Shared`] (safe to run in
	/// parallel with other shared tools).
	fn concurrency(&self) -> Concurrency {
		Concurrency::Shared
	}

	/// Execute the tool with the given input.
	///
	/// The optional `on_update` callback streams incremental output chunks
	/// to the UI during execution (e.g. for long-running bash commands).
	async fn execute(
		&self,
		input: &serde_json::Value,
		cwd: &Path,
		cancel: &CancellationToken,
		on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput>;
}
