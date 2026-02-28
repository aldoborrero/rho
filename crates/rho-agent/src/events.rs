use std::sync::Arc;

use crate::types::{AssistantMessage, Usage};

/// Events emitted by the agent loop for UI consumption.
#[derive(Debug, Clone)]
pub enum AgentEvent {
	/// A new tool-execution turn is starting.
	TurnStart { turn: u32 },
	/// Streaming text delta from the LLM.
	TextDelta(String),
	/// Streaming thinking delta from the LLM.
	ThinkingDelta(String),
	/// A tool call is about to be executed.
	ToolCallStart { id: String, name: String },
	/// A tool call completed.
	ToolCallResult { id: String, is_error: bool },
	/// The assistant message is complete (for session persistence).
	MessageComplete(AssistantMessage),
	/// A tool result message was created (for session persistence).
	ToolResultComplete { tool_use_id: String, content: Arc<String>, is_error: bool },
	/// A retry is scheduled after a transient error.
	RetryScheduled { attempt: u32, delay_ms: u64, error: String },
	/// The agent loop completed.
	Done(AgentOutcome),
}

/// How the agent loop concluded.
#[derive(Debug, Clone)]
pub enum AgentOutcome {
	/// Normal completion (`end_turn` or `stop_sequence`).
	Stop { usage: Option<Usage> },
	/// Hit `max_tokens` limit.
	MaxTokens { usage: Option<Usage> },
	/// Failed after exhausting retries.
	Failed { error: String },
	/// Cancelled by the caller.
	Cancelled,
}
