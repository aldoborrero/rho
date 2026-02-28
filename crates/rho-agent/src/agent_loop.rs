use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use futures_util::future::join_all;

use crate::{
	convert,
	events::{AgentEvent, AgentOutcome},
	registry::ToolRegistry,
	tools::{Concurrency, OnToolUpdate},
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
	pub system_prompt: Arc<String>,
	pub max_tokens:    u32,
	pub thinking:      ThinkingLevel,
	pub retry:         rho_ai::RetryConfig,
	pub cwd:           PathBuf,
	/// API key override (passed through to `StreamOptions`).
	pub api_key:       Option<String>,
	/// Temperature override. `None` or negative = provider default.
	pub temperature:   Option<f32>,
	/// Cancellation token — when cancelled, the loop exits immediately
	/// with `AgentOutcome::Cancelled`. Cancellation is raced against LLM
	/// streaming, tool execution, and retry delays via `tokio::select!`.
	pub abort:         Option<CancellationToken>,
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

	// Pre-convert tool definitions (immutable across turns).
	let mut ai_tools = convert::to_ai_tool_defs(&tools.definitions());
	// Pre-convert the initial message history; new messages are appended
	// incrementally so we never re-convert the full history.
	let mut ai_messages = convert::to_ai_messages(messages);

	// Build StreamOptions once — all fields are invariant across turns.
	let temperature = config.temperature.filter(|&t| t >= 0.0);
	let options = rho_ai::types::StreamOptions {
		api_key:     config.api_key.clone(),
		max_tokens:  Some(max_tokens_for(config.thinking, config.max_tokens)),
		reasoning:   thinking_to_reasoning(config.thinking),
		temperature,
		retry:       config.retry.clone(),
		abort:       config.abort.clone(),
		..Default::default()
	};

	// A token that never fires — used when config.abort is None.
	let cancel = config.abort.clone().unwrap_or_default();

	loop {
		// Checkpoint: before starting a new turn.
		if let Some(outcome) = check_should_stop(&cancel, &event_tx).await {
			return outcome;
		}

		turn += 1;
		let _ = event_tx.send(AgentEvent::TurnStart { turn }).await;

		// Build context by temporarily swapping the cached vecs in (zero
		// allocation). `rho_ai::stream` serialises the context into the HTTP
		// request body immediately; the returned `AssistantMessageStream`
		// (an mpsc receiver) does not borrow it.
		let mut context = rho_ai::types::Context {
			system_prompt: Some(config.system_prompt.clone()),
			messages:      std::mem::take(&mut ai_messages),
			tools:         std::mem::take(&mut ai_tools),
		};

		let stream = rho_ai::stream(model, &context, &options);

		// Reclaim the vecs — stream no longer borrows context.
		ai_messages = std::mem::take(&mut context.messages);
		ai_tools = std::mem::take(&mut context.tools);
		drop(context);

		let mut event_stream = stream.into_stream();

		let mut stream_error = false;
		let mut done_agent_msg: Option<crate::types::AssistantMessage> = None;
		let mut done_ai_msg: Option<rho_ai::types::AssistantMessage> = None;

		loop {
			let event = tokio::select! {
				event = event_stream.next() => match event {
					Some(e) => e,
					None => break,
				},
				() = cancel.cancelled() => {
					return emit_done(AgentOutcome::Cancelled, &event_tx).await;
				}
			};
			match event {
				rho_ai::StreamEvent::TextDelta { text, .. } => {
					let _ = event_tx.send(AgentEvent::TextDelta(text)).await;
				},
				rho_ai::StreamEvent::ThinkingDelta { thinking, .. } => {
					let _ = event_tx.send(AgentEvent::ThinkingDelta(thinking)).await;
				},
				rho_ai::StreamEvent::Done { message, .. } => {
					done_agent_msg = Some(convert::from_ai_assistant(&message));
					done_ai_msg = Some(message);
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
						// Race retry delay against cancellation.
						tokio::select! {
							() = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {},
							() = cancel.cancelled() => {
								return emit_done(AgentOutcome::Cancelled, &event_tx).await;
							}
						}
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
		let (Some(message), Some(ai_msg)) = (done_agent_msg, done_ai_msg) else {
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

		// Append assistant message to both histories. Push the original
		// ai-format message directly — no redundant agent→ai reconversion.
		messages.push(Message::Assistant(message.clone()));
		ai_messages.push(rho_ai::types::Message::Assistant(ai_msg));

		if has_tool_calls {
			// Collect tool-use blocks with their original index for ordered
			// result emission.
			let tool_uses: Vec<(usize, &str, &str, &serde_json::Value)> = message
				.content
				.iter()
				.enumerate()
				.filter_map(|(idx, b)| match b {
					ContentBlock::ToolUse { id, name, input } => {
						Some((idx, id.as_str(), name.as_str(), input))
					},
					_ => None,
				})
				.collect();

			// Checkpoint: before executing any tools.
			if let Some(outcome) = check_should_stop(&cancel, &event_tx).await {
				return outcome;
			}

			// Emit all ToolCallStart events upfront.
			for &(_, id, name, _) in &tool_uses {
				let _ = event_tx
					.send(AgentEvent::ToolCallStart {
						id:   id.to_owned(),
						name: name.to_owned(),
					})
					.await;
			}

			// Execute with barrier scheduling: shared tools run in
			// parallel, exclusive tools flush all pending work first,
			// then run alone. Each execution is raced against the
			// cancellation token for immediate abort.
			let mut results: Vec<Option<(Arc<String>, bool)>> = vec![None; tool_uses.len()];
			let mut pending_shared: Vec<(usize, &str, &str, &serde_json::Value)> = Vec::new();
			let mut cancelled = false;

			for (batch_idx, &(_, id, name, input)) in tool_uses.iter().enumerate() {
				if tools.concurrency(name) == Concurrency::Shared {
					pending_shared.push((batch_idx, id, name, input));
				} else {
					// Barrier: flush all pending shared tools first.
					if !pending_shared.is_empty() {
						let batch = std::mem::take(&mut pending_shared);
						if flush_shared_batch(&batch, tools, &config.cwd, &cancel, &event_tx, &mut results)
							.await
						{
							cancelled = true;
							break;
						}
					}
					// Execute exclusive tool alone, raced against cancellation.
					let cb = make_update_callback(&event_tx, id);
					let result = tokio::select! {
						result = tools.execute(name, input.clone(), &config.cwd, &cancel, Some(&cb)) => {
							wrap_tool_result(result)
						}
						() = cancel.cancelled() => {
							cancelled = true;
							break;
						}
					};
					results[batch_idx] = Some(result);
				}
			}

			// Flush remaining shared tools (unless already cancelled).
			if !cancelled && !pending_shared.is_empty() {
				let batch = std::mem::take(&mut pending_shared);
				if flush_shared_batch(&batch, tools, &config.cwd, &cancel, &event_tx, &mut results)
					.await
				{
					cancelled = true;
				}
			}

			if cancelled {
				return emit_done(AgentOutcome::Cancelled, &event_tx).await;
			}

			// Emit results and append to histories in original order.
			for (batch_idx, &(_, id, _, _)) in tool_uses.iter().enumerate() {
				let (content, is_error) = results[batch_idx]
					.take()
					.expect("all tool results should be populated");

				let _ = event_tx
					.send(AgentEvent::ToolCallResult {
						id: id.to_owned(),
						is_error,
					})
					.await;

				let _ = event_tx
					.send(AgentEvent::ToolResultComplete {
						tool_use_id: id.to_owned(),
						content:     Arc::clone(&content),
						is_error,
					})
					.await;

				let tool_msg = Message::ToolResult(ToolResultMessage {
					tool_use_id: id.to_owned(),
					content,
					is_error,
				});
				convert::push_ai_message(&mut ai_messages, &tool_msg);
				messages.push(tool_msg);
			}

			// Checkpoint: before sending tool results back to LLM.
			if let Some(outcome) = check_should_stop(&cancel, &event_tx).await {
				return outcome;
			}
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

/// Flush a batch of pending shared tools: create per-tool update callbacks,
/// run all in parallel via `join_all`, and store the results. Returns `true`
/// if cancellation fired before completion.
#[allow(clippy::future_not_send, reason = "ToolRegistry contains non-Send tools")]
async fn flush_shared_batch(
	batch: &[(usize, &str, &str, &serde_json::Value)],
	tools: &ToolRegistry,
	cwd: &std::path::Path,
	cancel: &CancellationToken,
	event_tx: &mpsc::Sender<AgentEvent>,
	results: &mut [Option<(Arc<String>, bool)>],
) -> bool {
	let callbacks: Vec<OnToolUpdate> = batch
		.iter()
		.map(|&(_, tid, _, _)| make_update_callback(event_tx, tid))
		.collect();
	let futures = batch.iter().zip(callbacks.iter()).map(|(&(_, _, n, inp), cb)| {
		tools.execute(n, inp.clone(), cwd, cancel, Some(cb))
	});
	tokio::select! {
		batch_results = join_all(futures) => {
			for (&(bi, ..), result) in batch.iter().zip(batch_results) {
				results[bi] = Some(wrap_tool_result(result));
			}
			false
		}
		() = cancel.cancelled() => true,
	}
}

/// Create an [`OnToolUpdate`] callback that forwards chunks to the event
/// channel as [`AgentEvent::ToolExecutionUpdate`].
///
/// Uses `try_send` (non-blocking) so the synchronous callback never blocks
/// a tokio worker thread. If the channel is full the update is silently
/// dropped (acceptable for UI streaming).
fn make_update_callback(
	event_tx: &mpsc::Sender<AgentEvent>,
	tool_use_id: &str,
) -> OnToolUpdate {
	let tx = event_tx.clone();
	let id = tool_use_id.to_owned();
	std::sync::Arc::new(move |content: &str| {
		let _ = tx.try_send(AgentEvent::ToolExecutionUpdate {
			id:      id.clone(),
			content: content.to_owned(),
		});
	})
}

/// Check if the loop should stop — either the abort token is cancelled or the
/// event channel has been closed (consumer dropped).
async fn check_should_stop(
	abort: &CancellationToken,
	event_tx: &mpsc::Sender<AgentEvent>,
) -> Option<AgentOutcome> {
	if abort.is_cancelled() {
		let outcome = AgentOutcome::Cancelled;
		let _ = event_tx.send(AgentEvent::Done(outcome.clone())).await;
		return Some(outcome);
	}
	if event_tx.is_closed() {
		return Some(AgentOutcome::Failed {
			error: "Event channel closed".to_owned(),
		});
	}
	None
}

/// Convert a tool execution result into an `(Arc<String>, is_error)` pair.
fn wrap_tool_result(result: anyhow::Result<crate::tools::ToolOutput>) -> (Arc<String>, bool) {
	match result {
		Ok(output) => (Arc::new(output.content), output.is_error),
		Err(e) => (Arc::new(format!("Tool execution error: {e}")), true),
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
