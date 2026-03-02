use std::{path::PathBuf, sync::Arc};

use futures_util::future::join_all;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::{
	convert,
	events::{AgentEvent, AgentOutcome},
	registry::ToolRegistry,
	tools::{Concurrency, MessageFetcher, OnToolUpdate},
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
	pub system_prompt:     Arc<String>,
	pub max_tokens:        u32,
	pub thinking:          ThinkingLevel,
	pub retry:             rho_ai::RetryConfig,
	pub cwd:               PathBuf,
	/// API key override (passed through to `StreamOptions`).
	pub api_key:           Option<String>,
	/// Temperature override. `None` or negative = provider default.
	pub temperature:       Option<f32>,
	/// Cancellation token — when cancelled, the loop exits immediately
	/// with `AgentOutcome::Cancelled`. Cancellation is raced against LLM
	/// streaming, tool execution, and retry delays via `tokio::select!`.
	pub abort:             Option<CancellationToken>,
	/// Polled at tool execution boundaries. Returns steering messages that
	/// interrupt the current tool batch — unexecuted tools are skipped.
	pub steering_fetcher:  Option<MessageFetcher>,
	/// Polled when the inner loop has no more tool calls or steering. Returns
	/// follow-up messages that start a new outer-loop iteration.
	pub follow_up_fetcher: Option<MessageFetcher>,
}

/// Outcome of executing a batch of tool calls with barrier scheduling.
struct ToolExecutionOutcome {
	/// Tool results indexed by position. `None` entries were skipped/cancelled.
	results:   Vec<Option<(Arc<String>, bool)>>,
	/// Steering messages that arrived during execution, causing remaining
	/// tools to be skipped. `None` if no steering occurred.
	steering:  Option<Vec<Message>>,
	/// Whether cancellation fired during execution.
	cancelled: bool,
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
		api_key: config.api_key.clone(),
		max_tokens: Some(max_tokens_for(config.thinking, config.max_tokens)),
		reasoning: thinking_to_reasoning(config.thinking),
		temperature,
		retry: config.retry.clone(),
		abort: config.abort.clone(),
		..Default::default()
	};

	// A token that never fires — used when config.abort is None.
	let cancel = config.abort.clone().unwrap_or_default();

	// OUTER LOOP: follow-up messages (autonomous continuation).
	// When `follow_up_fetcher` is `None`, this loop executes exactly once.
	loop {
		// Drain any steering messages queued while the agent was idle.
		let mut pending_messages = drain_messages(config.steering_fetcher.as_ref());

		// Carries the outcome from the inner loop to the follow-up check.
		let inner_outcome: AgentOutcome;

		// INNER LOOP: steering + tool execution.
		loop {
			// Inject pending messages (steering or follow-up) into history.
			if !pending_messages.is_empty() {
				for msg in &pending_messages {
					convert::push_ai_message(&mut ai_messages, msg);
					messages.push(msg.clone());
				}
				pending_messages.clear();
			}

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
							let delay = rho_ai::retry::calculate_backoff(
								&config.retry,
								retry_attempt,
								retry_after_ms,
							);
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

			// If we broke out of stream due to retry, continue the inner loop
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

			// Reset retry on success
			retry_attempt = 0;

			// Extract data we need *before* moving the message into
			// `messages.push()`, saving one deep clone per turn.
			let stop_reason = message.stop_reason.clone();
			let tool_uses: Vec<(usize, String, String, serde_json::Value)> = message
				.content
				.iter()
				.enumerate()
				.filter_map(|(idx, b)| match b {
					ContentBlock::ToolUse { id, name, input } => {
						Some((idx, id.clone(), name.clone(), input.clone()))
					},
					_ => None,
				})
				.collect();

			// Notify message complete (clone for event channel).
			let _ = event_tx
				.send(AgentEvent::MessageComplete(message.clone()))
				.await;

			// Append assistant message to both histories. Push the original
			// ai-format message directly — no redundant agent->ai reconversion.
			// Move instead of clone — tool data is already extracted above.
			messages.push(Message::Assistant(message));
			ai_messages.push(rho_ai::types::Message::Assistant(ai_msg));

			if !tool_uses.is_empty() {
				// Checkpoint: before executing any tools.
				if let Some(outcome) = check_should_stop(&cancel, &event_tx).await {
					return outcome;
				}

				// Emit all ToolCallStart events upfront.
				for (_, id, name, _) in &tool_uses {
					let _ = event_tx
						.send(AgentEvent::ToolCallStart { id: id.clone(), name: name.clone() })
						.await;
				}

				// Execute with barrier scheduling + steering poll points.
				let mut outcome = execute_tool_calls(
					&tool_uses,
					tools,
					&config.cwd,
					&cancel,
					&event_tx,
					config.steering_fetcher.as_ref(),
				)
				.await;

				if outcome.cancelled {
					return emit_done(AgentOutcome::Cancelled, &event_tx).await;
				}

				// Emit results and append to histories in original order.
				for (batch_idx, (_, id, ..)) in tool_uses.iter().enumerate() {
					let (content, is_error) = match outcome.results[batch_idx].take() {
						Some(r) => r,
						None if outcome.steering.is_some() => skip_tool_result(),
						None => (Arc::new("Tool execution aborted".to_owned()), true),
					};

					let _ = event_tx
						.send(AgentEvent::ToolCallResult { id: id.clone(), is_error })
						.await;

					let _ = event_tx
						.send(AgentEvent::ToolResultComplete {
							tool_use_id: id.clone(),
							content: Arc::clone(&content),
							is_error,
						})
						.await;

					let tool_msg = Message::ToolResult(ToolResultMessage {
						tool_use_id: id.clone(),
						content,
						is_error,
					});
					convert::push_ai_message(&mut ai_messages, &tool_msg);
					messages.push(tool_msg);
				}

				// If steering arrived, inject those messages and continue the inner loop.
				if let Some(steering_msgs) = outcome.steering {
					for msg in &steering_msgs {
						convert::push_ai_message(&mut ai_messages, msg);
						messages.push(msg.clone());
					}
					// Checkpoint before next turn.
					if let Some(o) = check_should_stop(&cancel, &event_tx).await {
						return o;
					}
					continue;
				}

				// Checkpoint: before sending tool results back to LLM.
				if let Some(outcome) = check_should_stop(&cancel, &event_tx).await {
					return outcome;
				}
				continue;
			}

			// No tool calls — compute outcome and break inner loop.
			// Clone usage so cumulative_usage remains available if the
			// outer loop continues with follow-up messages.
			inner_outcome = match stop_reason.as_ref() {
				Some(crate::types::StopReason::MaxTokens) => {
					AgentOutcome::MaxTokens { usage: cumulative_usage.clone() }
				},
				_ => {
					// EndTurn, StopSequence, or None — agent is done (for now).
					AgentOutcome::Stop { usage: cumulative_usage.clone() }
				},
			};
			break; // break inner loop — check follow-up in outer loop
		}

		// Inner loop done. Check for follow-up messages before returning.
		let follow_up = drain_messages(config.follow_up_fetcher.as_ref());
		if follow_up.is_empty() {
			// No follow-up — agent is truly done.
			return emit_done(inner_outcome, &event_tx).await;
		}

		// Follow-up messages present — inject into history and continue
		// the outer loop to start a new inner-loop iteration.
		for msg in &follow_up {
			convert::push_ai_message(&mut ai_messages, msg);
			messages.push(msg.clone());
		}
		// Continue outer loop — will drain steering at top and enter inner again.
	}
}

/// Execute a batch of tool calls using barrier scheduling with steering poll
/// points. Shared tools are batched for parallel execution; exclusive tools
/// flush pending shared work first and run alone. At each scheduling boundary,
/// steering messages are polled — if any arrive, remaining tools are skipped.
#[allow(clippy::future_not_send, reason = "ToolRegistry contains non-Send tools")]
async fn execute_tool_calls(
	tool_uses: &[(usize, String, String, serde_json::Value)],
	tools: &ToolRegistry,
	cwd: &std::path::Path,
	cancel: &CancellationToken,
	event_tx: &mpsc::Sender<AgentEvent>,
	steering_fetcher: Option<&MessageFetcher>,
) -> ToolExecutionOutcome {
	let mut results: Vec<Option<(Arc<String>, bool)>> = vec![None; tool_uses.len()];
	let mut pending_shared: Vec<(usize, &str, &str, &serde_json::Value)> = Vec::new();
	let mut cancelled = false;
	let mut steering: Option<Vec<Message>> = None;

	for (batch_idx, (_, id, name, input)) in tool_uses.iter().enumerate() {
		if tools.concurrency(name) == Concurrency::Shared {
			pending_shared.push((batch_idx, id.as_str(), name.as_str(), input));
		} else {
			// POLL 1: before flushing shared batch.
			let msgs = drain_messages(steering_fetcher);
			if !msgs.is_empty() {
				steering = Some(msgs);
				break;
			}

			// Barrier: flush all pending shared tools first.
			if !pending_shared.is_empty() {
				let batch = std::mem::take(&mut pending_shared);
				if flush_shared_batch(&batch, tools, cwd, cancel, event_tx, &mut results).await {
					cancelled = true;
					break;
				}
			}

			// POLL 2: before expensive exclusive tool.
			let msgs = drain_messages(steering_fetcher);
			if !msgs.is_empty() {
				steering = Some(msgs);
				break;
			}

			// Execute exclusive tool alone, raced against cancellation.
			let cb = make_update_callback(event_tx, id);
			let result = tokio::select! {
				result = tools.execute(name, input, cwd, cancel, Some(&cb)) => {
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

	// Flush remaining shared tools (unless cancelled or steered).
	if !cancelled && steering.is_none() && !pending_shared.is_empty() {
		// POLL 3: before final shared flush.
		let msgs = drain_messages(steering_fetcher);
		if msgs.is_empty() {
			let batch = std::mem::take(&mut pending_shared);
			if flush_shared_batch(&batch, tools, cwd, cancel, event_tx, &mut results).await {
				cancelled = true;
			}
		} else {
			steering = Some(msgs);
		}
	}

	// POLL 4: after all execution (catches messages that arrived during last tool).
	if !cancelled && steering.is_none() {
		let msgs = drain_messages(steering_fetcher);
		if !msgs.is_empty() {
			steering = Some(msgs);
		}
	}

	ToolExecutionOutcome { results, steering, cancelled }
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
		.map(|&(_, tid, ..)| make_update_callback(event_tx, tid))
		.collect();
	let futures = batch
		.iter()
		.zip(callbacks.iter())
		.map(|(&(_, _, n, inp), cb)| tools.execute(n, inp, cwd, cancel, Some(cb)));
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
fn make_update_callback(event_tx: &mpsc::Sender<AgentEvent>, tool_use_id: &str) -> OnToolUpdate {
	let tx = event_tx.clone();
	let id = tool_use_id.to_owned();
	std::sync::Arc::new(move |content: &str| {
		let _ = tx.try_send(AgentEvent::ToolExecutionUpdate {
			id:      id.clone(),
			content: content.to_owned(),
		});
	})
}

/// Drain messages from an optional fetcher. Returns an empty vec if the
/// fetcher is `None` or returns no messages.
fn drain_messages(fetcher: Option<&MessageFetcher>) -> Vec<Message> {
	match fetcher {
		Some(f) => f(),
		None => Vec::new(),
	}
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
		return Some(AgentOutcome::Failed { error: "Event channel closed".to_owned() });
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

/// Produce a placeholder result for a tool that was skipped due to a
/// steering message arriving mid-batch. Preserves the `tool_use` →
/// `tool_result` pairing invariant the LLM expects.
fn skip_tool_result() -> (Arc<String>, bool) {
	(Arc::new("Skipped due to queued user message.".to_owned()), true)
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

	#[test]
	fn drain_steering_returns_empty_when_no_fetcher() {
		let result = drain_messages(None);
		assert!(result.is_empty());
	}

	#[test]
	fn drain_steering_returns_messages_from_fetcher() {
		use crate::types::UserMessage;
		let fetcher: MessageFetcher = std::sync::Arc::new(|| {
			vec![Message::User(UserMessage { content: "steer me".to_owned() })]
		});
		let result = drain_messages(Some(&fetcher));
		assert_eq!(result.len(), 1);
		match &result[0] {
			Message::User(u) => assert_eq!(u.content, "steer me"),
			_ => panic!("expected User message"),
		}
	}

	#[test]
	fn skip_tool_result_produces_error_with_skip_message() {
		let (content, is_error) = skip_tool_result();
		assert!(is_error);
		assert_eq!(*content, "Skipped due to queued user message.");
	}

	#[test]
	fn drain_steering_fetcher_called_once_returns_empty_second_time() {
		use std::sync::atomic::{AtomicBool, Ordering};

		use crate::types::UserMessage;
		let called = std::sync::Arc::new(AtomicBool::new(false));
		let called2 = std::sync::Arc::clone(&called);
		let fetcher: MessageFetcher = std::sync::Arc::new(move || {
			if !called2.swap(true, Ordering::SeqCst) {
				vec![Message::User(UserMessage { content: "once".to_owned() })]
			} else {
				vec![]
			}
		});
		let f = Some(fetcher);
		assert_eq!(drain_messages(f.as_ref()).len(), 1);
		assert!(drain_messages(f.as_ref()).is_empty());
	}

	// --- Integration tests for execute_tool_calls with steering ---

	/// A mock tool that always succeeds, returning "<name> ran".
	/// Uses `Concurrency::Exclusive` so each tool gets its own scheduling
	/// barrier with poll points before execution.
	struct MockExclusiveTool {
		name: &'static str,
	}

	#[async_trait::async_trait]
	impl crate::tools::Tool for MockExclusiveTool {
		fn name(&self) -> &'static str {
			self.name
		}

		fn description(&self) -> &'static str {
			"mock"
		}

		fn input_schema(&self) -> serde_json::Value {
			serde_json::json!({})
		}

		fn concurrency(&self) -> crate::tools::Concurrency {
			crate::tools::Concurrency::Exclusive
		}

		async fn execute(
			&self,
			_input: &serde_json::Value,
			_cwd: &std::path::Path,
			_cancel: &tokio_util::sync::CancellationToken,
			_on_update: Option<&crate::tools::OnToolUpdate>,
		) -> anyhow::Result<crate::tools::ToolOutput> {
			Ok(crate::tools::ToolOutput { content: format!("{} ran", self.name), is_error: false })
		}
	}

	/// Build a `ToolRegistry` containing two exclusive mock tools.
	fn mock_registry() -> ToolRegistry {
		let mut builder = crate::registry::ToolRegistryBuilder::new();
		builder.register(Box::new(MockExclusiveTool { name: "tool_a" }));
		builder.register(Box::new(MockExclusiveTool { name: "tool_b" }));
		builder.build()
	}

	/// Build the `tool_uses` input for two tools.
	fn two_tool_uses() -> Vec<(usize, String, String, serde_json::Value)> {
		vec![
			(0, "call_0".to_owned(), "tool_a".to_owned(), serde_json::json!({})),
			(1, "call_1".to_owned(), "tool_b".to_owned(), serde_json::json!({})),
		]
	}

	/// When a steering fetcher fires after `tool_a` executes but before
	/// `tool_b`, the outcome should have `steering` set and `tool_b`'s
	/// result should be `None` (skipped).
	///
	/// Poll sequence for two exclusive tools:
	///   `tool_a`: POLL 1 (n=0, empty), POLL 2 (n=1, empty), execute
	///   `tool_b`: POLL 1 (n=2, fires!), break
	#[tokio::test]
	async fn execute_tool_calls_skips_tools_on_steering() {
		use std::sync::atomic::{AtomicU32, Ordering};

		use crate::types::UserMessage;

		let tools = mock_registry();
		let tool_uses = two_tool_uses();

		// Fires on the 3rd drain_messages call (n >= 2), which is the
		// POLL 1 check before tool_b.
		let poll_count = std::sync::Arc::new(AtomicU32::new(0));
		let pc = std::sync::Arc::clone(&poll_count);
		let fetcher: MessageFetcher = std::sync::Arc::new(move || {
			let n = pc.fetch_add(1, Ordering::SeqCst);
			if n >= 2 {
				vec![Message::User(UserMessage { content: "redirect".to_owned() })]
			} else {
				vec![]
			}
		});

		let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
		let cancel = tokio_util::sync::CancellationToken::new();

		let outcome = execute_tool_calls(
			&tool_uses,
			&tools,
			std::path::Path::new("/tmp"),
			&cancel,
			&event_tx,
			Some(&fetcher),
		)
		.await;

		// Steering should have fired.
		assert!(outcome.steering.is_some(), "expected steering to be set");
		let steering_msgs = outcome.steering.unwrap();
		assert_eq!(steering_msgs.len(), 1);
		match &steering_msgs[0] {
			Message::User(u) => assert_eq!(u.content, "redirect"),
			other => panic!("expected User message, got: {other:?}"),
		}

		// tool_a should have executed successfully.
		assert!(outcome.results[0].is_some(), "tool_a should have run");
		let (content, is_error) = outcome.results[0].as_ref().unwrap();
		assert_eq!(content.as_str(), "tool_a ran");
		assert!(!is_error);

		// tool_b should have been skipped (None).
		assert!(outcome.results[1].is_none(), "tool_b should have been skipped by steering");

		// Not cancelled.
		assert!(!outcome.cancelled);
	}

	/// When no steering messages arrive, all tools should execute and return
	/// `Some` results.
	#[tokio::test]
	async fn execute_tool_calls_no_steering_returns_all_results() {
		let tools = mock_registry();
		let tool_uses = two_tool_uses();

		// Fetcher that never returns any messages.
		let fetcher: MessageFetcher = std::sync::Arc::new(Vec::new);

		let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
		let cancel = tokio_util::sync::CancellationToken::new();

		let outcome = execute_tool_calls(
			&tool_uses,
			&tools,
			std::path::Path::new("/tmp"),
			&cancel,
			&event_tx,
			Some(&fetcher),
		)
		.await;

		// No steering should have fired.
		assert!(outcome.steering.is_none(), "expected no steering");

		// Both tools should have executed.
		assert!(outcome.results[0].is_some(), "tool_a should have run");
		let (content_a, is_error_a) = outcome.results[0].as_ref().unwrap();
		assert_eq!(content_a.as_str(), "tool_a ran");
		assert!(!is_error_a);

		assert!(outcome.results[1].is_some(), "tool_b should have run");
		let (content_b, is_error_b) = outcome.results[1].as_ref().unwrap();
		assert_eq!(content_b.as_str(), "tool_b ran");
		assert!(!is_error_b);

		// Not cancelled.
		assert!(!outcome.cancelled);
	}
}
