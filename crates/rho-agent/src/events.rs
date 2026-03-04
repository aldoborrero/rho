use std::sync::Arc;

use crate::types::{AssistantMessage, Message, ToolResultMessage, Usage, UserMessage};

/// Events emitted by the agent loop for UI consumption.
#[derive(Debug, Clone)]
pub enum AgentEvent {
	/// The agent loop has begun.
	AgentStart,
	/// A new tool-execution turn is starting.
	TurnStart { turn: u32 },
	/// The assistant message stream is starting (first content arriving).
	MessageStart,
	/// Streaming text delta from the LLM.
	TextDelta(String),
	/// Streaming thinking delta from the LLM.
	ThinkingDelta(String),
	/// A tool call is about to be executed.
	ToolCallStart { id: String, name: String, input: Arc<serde_json::Value> },
	/// Incremental output from a running tool (for real-time UI updates).
	ToolExecutionUpdate { id: String, name: String, content: String },
	/// A tool call completed (UI lifecycle + session persistence).
	ToolCallResult { id: String, name: String, content: Arc<String>, is_error: bool },
	/// A turn has completed with its assistant message and tool results.
	TurnEnd { message: Arc<AssistantMessage>, tool_results: Vec<ToolResultMessage> },
	/// The assistant message is complete (for session persistence).
	MessageComplete(Arc<AssistantMessage>),
	/// Steering messages from the user were injected into the conversation.
	SteeringProcessed { messages: Vec<UserMessage> },
	/// A retry is scheduled after a transient error.
	RetryScheduled { attempt: u32, delay_ms: u64, error: String },
	/// The agent loop completed, with all messages produced during this run.
	Done { outcome: AgentOutcome, messages: Arc<[Message]> },
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
