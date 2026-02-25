use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::{
	convert,
	events::{AgentEvent, AgentOutcome},
	registry::ToolRegistry,
	types::{ContentBlock, Message, ToolResultMessage, Usage},
};

/// Thinking budget level.
#[derive(Debug, Clone, Copy, Default)]
pub enum ThinkingLevel {
	#[default]
	Off,
	Low,
	Medium,
	High,
}

/// Configuration for a single agent loop run.
pub struct AgentConfig {
	pub system_prompt: String,
	pub max_tokens:    u32,
	pub thinking:      ThinkingLevel,
	pub retry:         rho_ai::RetryConfig,
	pub cwd:           PathBuf,
	/// API key override (passed through to `StreamOptions`).
	pub api_key:       Option<String>,
	/// Temperature override. `None` or negative = provider default.
	pub temperature:   Option<f32>,
}

/// Run the autonomous agent loop.
///
/// Streams LLM responses, executes tool calls, retries on transient errors,
/// and emits [`AgentEvent`]s for UI consumption. Returns when the LLM signals
/// `end_turn`, hits `max_tokens`, or all retries are exhausted.
///
/// The caller owns `messages` and can persist them via the event stream
/// ([`AgentEvent::MessageComplete`], [`AgentEvent::ToolResultComplete`]).
#[allow(
	clippy::future_not_send,
	reason = "ToolRegistry contains non-Send tools; runs on main task"
)]
pub async fn run_agent_loop(
	model: &rho_ai::Model,
	messages: &mut Vec<Message>,
	tools: &ToolRegistry,
	config: AgentConfig,
	event_tx: mpsc::Sender<AgentEvent>,
) -> AgentOutcome {
	let mut turn: u32 = 0;
	let mut retry_attempt: u32 = 0;
	let mut cumulative_usage: Option<Usage> = None;

	loop {
		turn += 1;
		let _ = event_tx.send(AgentEvent::TurnStart { turn }).await;

		// Build rho-ai context
		let ai_messages = convert::to_ai_messages(messages);
		let ai_tools = convert::to_ai_tool_defs(&tools.definitions());
		let context = rho_ai::types::Context {
			system_prompt: Some(config.system_prompt.clone()),
			messages:      ai_messages,
			tools:         ai_tools,
		};

		let temperature = config
			.temperature
			.filter(|&t| t >= 0.0);
		let options = rho_ai::types::StreamOptions {
			api_key: config.api_key.clone(),
			max_tokens: Some(max_tokens_for(config.thinking, config.max_tokens)),
			reasoning: thinking_to_reasoning(config.thinking),
			temperature,
			retry: config.retry.clone(),
			..Default::default()
		};

		// Stream from rho-ai
		let stream = rho_ai::stream(model, &context, &options);
		let mut event_stream = stream.into_stream();

		let mut stream_error = false;
		let mut done_message: Option<crate::types::AssistantMessage> = None;

		while let Some(event) = event_stream.next().await {
			match event {
				rho_ai::StreamEvent::TextDelta { text, .. } => {
					let _ = event_tx.send(AgentEvent::TextDelta(text)).await;
				},
				rho_ai::StreamEvent::ThinkingDelta { thinking, .. } => {
					let _ = event_tx.send(AgentEvent::ThinkingDelta(thinking)).await;
				},
				rho_ai::StreamEvent::Done { message, .. } => {
					let agent_message = convert::from_ai_assistant(&message);
					done_message = Some(agent_message);
				},
				rho_ai::StreamEvent::Error { error, retryable, retry_after_ms } => {
					let error_msg = error.to_string();
					if retryable && config.retry.enabled && retry_attempt < config.retry.max_retries {
						retry_attempt += 1;
						let delay =
							rho_ai::retry::calculate_backoff(&config.retry, retry_attempt, retry_after_ms);
						let _ = event_tx
							.send(AgentEvent::RetryScheduled {
								attempt:  retry_attempt,
								delay_ms: delay,
								error:    error_msg,
							})
							.await;
						tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
						stream_error = true;
						break;
					}
					return emit_done(AgentOutcome::Failed { error: error_msg }, &event_tx).await;
				},
				// Other events (TextStart/End, ToolCallStart/Delta/End, ThinkingStart/End)
				_ => {},
			}
		}

		// If we broke out of stream due to retry, continue the outer loop
		if stream_error {
			continue;
		}

		// If we have no done message, the stream ended unexpectedly
		let Some(message) = done_message else {
			return emit_done(
				AgentOutcome::Failed { error: "Stream ended without a Done event".to_owned() },
				&event_tx,
			)
			.await;
		};

		// Merge usage
		if let Some(ref usage) = message.usage {
			cumulative_usage = Some(match cumulative_usage {
				Some(mut prev) => {
					prev.input_tokens += usage.input_tokens;
					prev.output_tokens += usage.output_tokens;
					prev
				},
				None => usage.clone(),
			});
		}

		// Notify message complete
		let _ = event_tx
			.send(AgentEvent::MessageComplete(message.clone()))
			.await;

		// Reset retry on success
		retry_attempt = 0;

		// Check for tool calls
		let has_tool_calls = message
			.content
			.iter()
			.any(|b| matches!(b, ContentBlock::ToolUse { .. }));

		// Append assistant message to context
		messages.push(Message::Assistant(message.clone()));

		if has_tool_calls {
			// Execute tools
			for block in &message.content {
				if let ContentBlock::ToolUse { id, name, input } = block {
					let _ = event_tx
						.send(AgentEvent::ToolCallStart { id: id.clone(), name: name.clone() })
						.await;

					let tool_result = tools.execute(name, input.clone(), &config.cwd).await;

					let (content, is_error) = match tool_result {
						Ok(output) => (output.content, output.is_error),
						Err(e) => (format!("Tool execution error: {e}"), true),
					};

					let _ = event_tx
						.send(AgentEvent::ToolCallResult {
							id: id.clone(),
							is_error,
							content: content.clone(),
						})
						.await;

					// Notify for session persistence
					let _ = event_tx
						.send(AgentEvent::ToolResultComplete {
							tool_use_id: id.clone(),
							content: content.clone(),
							is_error,
						})
						.await;

					// Append tool result to context
					messages.push(Message::ToolResult(ToolResultMessage {
						tool_use_id: id.clone(),
						content,
						is_error,
					}));
				}
			}
			// Loop back to send tool results to LLM
			continue;
		}

		// No tool calls — check terminal conditions
		let outcome = match message.stop_reason.as_ref() {
			Some(crate::types::StopReason::MaxTokens) => {
				AgentOutcome::MaxTokens { usage: cumulative_usage }
			},
			_ => {
				// EndTurn, StopSequence, or None — agent is done
				AgentOutcome::Stop { usage: cumulative_usage }
			},
		};
		return emit_done(outcome, &event_tx).await;
	}
}

/// Emit the [`AgentEvent::Done`] event and return the outcome.
async fn emit_done(outcome: AgentOutcome, event_tx: &mpsc::Sender<AgentEvent>) -> AgentOutcome {
	let _ = event_tx.send(AgentEvent::Done(outcome.clone())).await;
	outcome
}

const fn thinking_to_reasoning(level: ThinkingLevel) -> Option<rho_ai::types::ReasoningLevel> {
	match level {
		ThinkingLevel::Off => None,
		ThinkingLevel::Low => Some(rho_ai::types::ReasoningLevel::Low),
		ThinkingLevel::Medium => Some(rho_ai::types::ReasoningLevel::Medium),
		ThinkingLevel::High => Some(rho_ai::types::ReasoningLevel::High),
	}
}

const fn max_tokens_for(thinking: ThinkingLevel, default: u32) -> u32 {
	match thinking {
		ThinkingLevel::Off => default,
		_ => 16384,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_thinking_to_reasoning() {
		assert!(thinking_to_reasoning(ThinkingLevel::Off).is_none());

		assert!(matches!(
			thinking_to_reasoning(ThinkingLevel::Low),
			Some(rho_ai::types::ReasoningLevel::Low)
		));
		assert!(matches!(
			thinking_to_reasoning(ThinkingLevel::Medium),
			Some(rho_ai::types::ReasoningLevel::Medium)
		));
		assert!(matches!(
			thinking_to_reasoning(ThinkingLevel::High),
			Some(rho_ai::types::ReasoningLevel::High)
		));
	}

	#[test]
	fn test_max_tokens_for_thinking() {
		assert_eq!(max_tokens_for(ThinkingLevel::Off, 8192), 8192);
		assert_eq!(max_tokens_for(ThinkingLevel::Low, 8192), 16384);
		assert_eq!(max_tokens_for(ThinkingLevel::High, 8192), 16384);
	}
}
