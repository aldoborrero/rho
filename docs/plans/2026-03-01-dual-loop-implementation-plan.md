# Dual-Loop Two-Tier Queue Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add steering messages (mid-turn user input) and follow-up messages (autonomous continuation) to the agent loop using a poll-based two-tier queue.

**Architecture:** Fetcher callbacks (`Arc<dyn Fn() -> Vec<Message> + Send + Sync>`) passed via `AgentConfig`. The agent loop polls these at tool execution boundaries (steering) and after the inner loop exhausts (follow-up). Interactive mode owns backing `VecDeque`s behind `Arc<Mutex<>>`. Single-loop becomes nested dual-loop.

**Tech Stack:** Rust, tokio, `std::sync::Mutex`, `std::collections::VecDeque`

**Design doc:** `docs/plans/2026-03-01-dual-loop-two-tier-queue-design.md`

---

### Task 1: Add `MessageFetcher` type and update `AgentConfig`

**Files:**
- Modify: `crates/rho-agent/src/tools.rs` (after line 29)
- Modify: `crates/rho-agent/src/agent_loop.rs:29-43` (`AgentConfig`)
- Modify: `crates/rho/src/modes/interactive.rs:155-169` (`spawn_agent` — add `None` defaults)

**Step 1: Add `MessageFetcher` type alias to `tools.rs`**

In `crates/rho-agent/src/tools.rs`, after the `OnToolUpdate` type alias (line 29), add:

```rust
/// Synchronous callback that drains queued messages from the caller.
/// Returns an empty `Vec` when no messages are pending. Polled at tool
/// execution boundaries (steering) and after the inner loop exhausts
/// (follow-up). Uses `Fn` (not async) because the backing store is a
/// `Mutex<VecDeque>` — no await needed.
pub type MessageFetcher = Arc<dyn Fn() -> Vec<Message> + Send + Sync>;
```

This requires adding `use crate::types::Message;` to the imports in `tools.rs`.

**Step 2: Add fetcher fields to `AgentConfig`**

In `crates/rho-agent/src/agent_loop.rs`, add to `AgentConfig` (after the `abort` field, line 42):

```rust
	/// Polled at tool execution boundaries. Returns steering messages that
	/// interrupt the current tool batch — unexecuted tools are skipped.
	pub steering_fetcher:  Option<MessageFetcher>,
	/// Polled when the inner loop has no more tool calls or steering. Returns
	/// follow-up messages that start a new outer-loop iteration.
	pub follow_up_fetcher: Option<MessageFetcher>,
```

Update the import line (line 14) to include `MessageFetcher`:

```rust
	tools::{Concurrency, MessageFetcher, OnToolUpdate},
```

**Step 3: Fix compilation — add `None` defaults to all `AgentConfig` construction sites**

In `crates/rho/src/modes/interactive.rs`, `spawn_agent` function (around line 155), add to the `AgentConfig` struct literal:

```rust
		steering_fetcher:  None,
		follow_up_fetcher: None,
```

Search for any other `AgentConfig` construction sites:

```bash
rg 'AgentConfig\s*\{' crates/
```

Add `steering_fetcher: None, follow_up_fetcher: None,` to each.

**Step 4: Verify compilation**

Run: `cargo build --workspace`
Expected: compiles with zero errors.

**Step 5: Commit**

```bash
git add crates/rho-agent/src/tools.rs crates/rho-agent/src/agent_loop.rs crates/rho/src/modes/interactive.rs
git commit -m "feat: add MessageFetcher type and steering/follow-up fields to AgentConfig"
```

---

### Task 2: Add `drain_steering` and `drain_follow_up` helpers

**Files:**
- Modify: `crates/rho-agent/src/agent_loop.rs` (add helper functions near `check_should_stop`)

**Step 1: Write the test for `drain_steering`**

In `crates/rho-agent/src/agent_loop.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
	#[test]
	fn drain_steering_returns_empty_when_no_fetcher() {
		let result = drain_messages(&None);
		assert!(result.is_empty());
	}

	#[test]
	fn drain_steering_returns_messages_from_fetcher() {
		use crate::types::UserMessage;
		let fetcher: MessageFetcher = std::sync::Arc::new(|| {
			vec![Message::User(UserMessage { content: "steer me".to_owned() })]
		});
		let result = drain_messages(&Some(fetcher));
		assert_eq!(result.len(), 1);
		match &result[0] {
			Message::User(u) => assert_eq!(u.content, "steer me"),
			_ => panic!("expected User message"),
		}
	}

	#[test]
	fn drain_steering_fetcher_called_once_returns_empty_second_time() {
		use crate::types::UserMessage;
		use std::sync::atomic::{AtomicBool, Ordering};
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
		assert_eq!(drain_messages(&f).len(), 1);
		assert!(drain_messages(&f).is_empty());
	}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rho-agent -- drain_steering`
Expected: FAIL — `drain_messages` not found.

**Step 3: Implement `drain_messages`**

In `crates/rho-agent/src/agent_loop.rs`, add before `check_should_stop`:

```rust
/// Drain messages from an optional fetcher. Returns an empty vec if the
/// fetcher is `None` or returns no messages.
fn drain_messages(fetcher: &Option<MessageFetcher>) -> Vec<Message> {
	match fetcher {
		Some(f) => f(),
		None => Vec::new(),
	}
}
```

Also add `MessageFetcher` to the import from `crate::tools` if not already done in Task 1.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rho-agent -- drain_`
Expected: all 3 tests PASS.

**Step 5: Commit**

```bash
git add crates/rho-agent/src/agent_loop.rs
git commit -m "feat: add drain_messages helper for polling fetcher callbacks"
```

---

### Task 3: Add `skip_tool_result` helper

**Files:**
- Modify: `crates/rho-agent/src/agent_loop.rs` (add helper + tests)

**Step 1: Write the test**

In `crates/rho-agent/src/agent_loop.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
	#[test]
	fn skip_tool_result_produces_error_with_skip_message() {
		let (content, is_error) = skip_tool_result();
		assert!(is_error);
		assert_eq!(*content, "Skipped due to queued user message.");
	}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rho-agent -- skip_tool_result`
Expected: FAIL — `skip_tool_result` not found.

**Step 3: Implement `skip_tool_result`**

In `crates/rho-agent/src/agent_loop.rs`, add near `wrap_tool_result`:

```rust
/// Produce a placeholder result for a tool that was skipped due to a
/// steering message arriving mid-batch. Preserves the tool_use → tool_result
/// pairing invariant the LLM expects.
fn skip_tool_result() -> (Arc<String>, bool) {
	(Arc::new("Skipped due to queued user message.".to_owned()), true)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p rho-agent -- skip_tool_result`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/rho-agent/src/agent_loop.rs
git commit -m "feat: add skip_tool_result helper for steering-skipped tools"
```

---

### Task 4: Refactor tool execution into `execute_tool_calls` returning `ToolExecutionOutcome`

This is the largest task. It extracts the barrier scheduling block (current lines 247-322) into a separate function that returns a `ToolExecutionOutcome` carrying both results and optional steering messages.

**Files:**
- Modify: `crates/rho-agent/src/agent_loop.rs:227-328` (extract + modify barrier scheduler)

**Step 1: Define `ToolExecutionOutcome`**

Add above `run_agent_loop`:

```rust
/// Outcome of executing a batch of tool calls with barrier scheduling.
struct ToolExecutionOutcome {
	/// Tool results indexed by position. `None` entries were skipped/cancelled.
	results: Vec<Option<(Arc<String>, bool)>>,
	/// Steering messages that arrived during execution, causing remaining
	/// tools to be skipped. `None` if no steering occurred.
	steering: Option<Vec<Message>>,
	/// Whether cancellation fired during execution.
	cancelled: bool,
}
```

**Step 2: Extract `execute_tool_calls` function**

Create a new function that encapsulates the current lines 247-288 (barrier scheduler) plus steering poll points. The function signature:

```rust
#[allow(clippy::future_not_send, reason = "ToolRegistry contains non-Send tools")]
async fn execute_tool_calls(
	tool_uses: &[(usize, String, String, serde_json::Value)],
	tools: &ToolRegistry,
	cwd: &std::path::Path,
	cancel: &CancellationToken,
	event_tx: &mpsc::Sender<AgentEvent>,
	steering_fetcher: &Option<MessageFetcher>,
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
		if !msgs.is_empty() {
			steering = Some(msgs);
		} else {
			let batch = std::mem::take(&mut pending_shared);
			if flush_shared_batch(&batch, tools, cwd, cancel, event_tx, &mut results).await {
				cancelled = true;
			}
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
```

**Step 3: Update `run_agent_loop` to call `execute_tool_calls`**

Replace lines 247-292 (the barrier scheduler block) with:

```rust
			let outcome = execute_tool_calls(
				&tool_uses, tools, &config.cwd, &cancel, &event_tx, &config.steering_fetcher,
			).await;

			if outcome.cancelled {
				return emit_done(AgentOutcome::Cancelled, &event_tx).await;
			}
```

**Step 4: Update result emission to handle skipped tools**

Replace lines 294-322 (the result emission loop) with:

```rust
			// Emit results and append to histories in original order.
			for (batch_idx, (_, id, _, _)) in tool_uses.iter().enumerate() {
				let (content, is_error) = match outcome.results[batch_idx] {
					Some(ref r) => r.clone(),
					None if outcome.steering.is_some() => skip_tool_result(),
					None => (Arc::new("Tool execution aborted".to_owned()), true),
				};

				let _ = event_tx
					.send(AgentEvent::ToolCallResult {
						id: id.clone(),
						is_error,
					})
					.await;

				let _ = event_tx
					.send(AgentEvent::ToolResultComplete {
						tool_use_id: id.clone(),
						content:     Arc::clone(&content),
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
```

**Step 5: Verify compilation and tests**

Run: `cargo build --workspace && cargo test -p rho-agent`
Expected: compiles, all existing tests pass.

**Step 6: Commit**

```bash
git add crates/rho-agent/src/agent_loop.rs
git commit -m "refactor: extract execute_tool_calls with steering poll points"
```

---

### Task 5: Add the outer follow-up loop

**Files:**
- Modify: `crates/rho-agent/src/agent_loop.rs:89-342` (wrap existing loop)

**Step 1: Wrap the existing inner loop in an outer loop**

The current `loop { ... }` at line 89 becomes the inner loop. Add an outer loop around it and check follow-up after the inner loop breaks:

The structure becomes (showing only the structural changes, not full code):

```rust
	// OUTER LOOP: follow-up messages (autonomous continuation).
	'outer: loop {
		// Drain any steering messages queued while idle.
		let mut pending_messages = drain_messages(&config.steering_fetcher);

		// INNER LOOP: steering + tool execution.
		let mut last_outcome: Option<AgentOutcome> = None;
		loop {
			// If we have pending messages (steering or follow-up), inject them.
			if !pending_messages.is_empty() {
				for msg in &pending_messages {
					convert::push_ai_message(&mut ai_messages, msg);
					messages.push(msg.clone());
				}
				pending_messages.clear();
			}

			// ... existing turn logic (checkpoint, stream LLM, etc.) ...

			// When tool execution yields steering:
			if let Some(steering_msgs) = outcome.steering {
				pending_messages = steering_msgs;
				continue; // continue INNER loop
			}

			// No tool calls — record outcome and break inner loop.
			// (This replaces the current `return emit_done(...)`)
			last_outcome = Some(match stop_reason.as_ref() {
				Some(crate::types::StopReason::MaxTokens) => {
					AgentOutcome::MaxTokens { usage: cumulative_usage.clone() }
				},
				_ => AgentOutcome::Stop { usage: cumulative_usage.clone() },
			});
			break; // break INNER loop
		}

		// Inner loop done. Check follow-up messages.
		let follow_up = drain_messages(&config.follow_up_fetcher);
		if follow_up.is_empty() {
			// No follow-up — agent truly done.
			let outcome = last_outcome.unwrap_or(AgentOutcome::Stop { usage: cumulative_usage });
			return emit_done(outcome, &event_tx).await;
		}
		// Follow-up messages present — they become pending for the next outer iteration.
		pending_messages = follow_up;
		// The outer loop will inject them at the top of the inner loop.
		// But we need to re-enter inner, so we use a flag or restructure.
		// Simplest: the outer loop drains steering at top, so set these as
		// pending_messages handled at the start of the inner loop.
	}
```

Note: The `continue` for steering stays within the inner loop. The `break` for no-tool-calls exits the inner loop. The outer loop checks follow-up and either returns or continues.

**Step 2: Verify compilation and existing tests**

Run: `cargo build --workspace && cargo test --workspace`
Expected: compiles, all tests pass. Behavior is unchanged when fetchers are `None` (both `drain_messages` return empty vecs).

**Step 3: Commit**

```bash
git add crates/rho-agent/src/agent_loop.rs
git commit -m "feat: add outer follow-up loop to agent loop (dual-loop structure)"
```

---

### Task 6: Wire fetchers in interactive mode

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs:318-686`

**Step 1: Add queue storage to `run_interactive`**

Near the top of `run_interactive` (after existing state variables like `agent_cancel`, around line 430), add:

```rust
		let steering_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Message>>> =
			std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
		let follow_up_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Message>>> =
			std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
```

**Step 2: Update `spawn_agent` to accept and pass fetchers**

Add parameters to `spawn_agent`:

```rust
fn spawn_agent(
	model: &rho_ai::Model,
	messages: &[Message],
	tools: &ToolRegistry,
	system_prompt: &str,
	settings: &Settings,
	api_key: &str,
	tx: &tokio::sync::mpsc::Sender<AppEvent>,
	agent_generation: &mut u64,
	steering_fetcher: Option<MessageFetcher>,   // NEW
	follow_up_fetcher: Option<MessageFetcher>,  // NEW
) -> anyhow::Result<tokio_util::sync::CancellationToken> {
```

And pass them through to `AgentConfig`:

```rust
		steering_fetcher,
		follow_up_fetcher,
```

**Step 3: Build fetcher closures at call sites**

At each `spawn_agent(...)` call site, construct the fetchers:

```rust
	let sq = std::sync::Arc::clone(&steering_queue);
	let sf: Option<MessageFetcher> = Some(std::sync::Arc::new(move || {
		let mut q = sq.lock().unwrap_or_else(|e| e.into_inner());
		q.pop_front().into_iter().collect()
	}));
	let fq = std::sync::Arc::clone(&follow_up_queue);
	let ff: Option<MessageFetcher> = Some(std::sync::Arc::new(move || {
		let mut q = fq.lock().unwrap_or_else(|e| e.into_inner());
		q.pop_front().into_iter().collect()
	}));
```

Pass `sf` and `ff` to `spawn_agent`.

**Step 4: Verify compilation**

Run: `cargo build --workspace`
Expected: compiles. Behavior unchanged — queues are always empty.

**Step 5: Commit**

```bash
git add crates/rho/src/modes/interactive.rs
git commit -m "feat: wire MessageFetcher closures into spawn_agent"
```

---

### Task 7: Route user input to steering queue

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs:642-678` (the `InputAction::UserMessage` handler)

**Step 1: Change input routing when streaming**

Replace the current block at lines 642-678 that cancels the agent and respawns:

```rust
InputAction::UserMessage(text) => {
	if matches!(mode, AppMode::Streaming) {
		// Steer the running agent instead of cancelling.
		let user_msg = Message::User(UserMessage { content: text.to_owned() });
		app.chat.add_message(user_msg.clone());
		session.append(user_msg.clone()).await?;
		steering_queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(user_msg);
	} else {
		// Idle — existing behavior: spawn new agent.
		let user_msg = Message::User(UserMessage { content: text.to_owned() });
		app.chat.add_message(user_msg.clone());
		session.append(user_msg).await?;

		mode = AppMode::Streaming;
		app.chat.start_streaming();
		app.status.clear_work_status();
		app.status.start_working();
		app.update_status_border(terminal.columns());

		// Clear any stale messages from previous runs.
		steering_queue.lock().unwrap_or_else(|e| e.into_inner()).clear();
		follow_up_queue.lock().unwrap_or_else(|e| e.into_inner()).clear();

		let sq = std::sync::Arc::clone(&steering_queue);
		let sf: Option<MessageFetcher> = Some(std::sync::Arc::new(move || {
			let mut q = sq.lock().unwrap_or_else(|e| e.into_inner());
			q.pop_front().into_iter().collect()
		}));
		let fq = std::sync::Arc::clone(&follow_up_queue);
		let ff: Option<MessageFetcher> = Some(std::sync::Arc::new(move || {
			let mut q = fq.lock().unwrap_or_else(|e| e.into_inner());
			q.pop_front().into_iter().collect()
		}));

		agent_cancel = Some(spawn_agent(
			&model,
			session.messages(),
			&tools,
			&system_prompt,
			&settings,
			&api_key,
			&tx,
			&mut agent_generation,
			sf,
			ff,
		)?);
	}
},
```

**Step 2: Verify compilation**

Run: `cargo build --workspace`
Expected: compiles.

**Step 3: Commit**

```bash
git add crates/rho/src/modes/interactive.rs
git commit -m "feat: route user input to steering queue when agent is streaming"
```

---

### Task 8: Integration tests

**Files:**
- Modify: `crates/rho-agent/src/agent_loop.rs` (add integration tests)

**Step 1: Write integration test for steering**

This test uses a mock provider (or the existing test infrastructure) to verify that when a steering fetcher returns a message, unexecuted tools get skip results and the steering message appears in the final message history.

The exact test code depends on the test utilities available (mock `ToolRegistry`, mock LLM provider). At minimum, test the extracted `execute_tool_calls` function:

```rust
	#[tokio::test]
	async fn execute_tool_calls_skips_on_steering() {
		use crate::types::UserMessage;
		use std::sync::atomic::{AtomicU32, Ordering};

		// Create a steering fetcher that fires on the 2nd poll.
		let poll_count = std::sync::Arc::new(AtomicU32::new(0));
		let pc = std::sync::Arc::clone(&poll_count);
		let fetcher: MessageFetcher = std::sync::Arc::new(move || {
			let n = pc.fetch_add(1, Ordering::SeqCst);
			if n >= 1 {
				vec![Message::User(UserMessage { content: "redirect".to_owned() })]
			} else {
				vec![]
			}
		});

		// ... setup ToolRegistry with 2 exclusive tools, event channel, etc.
		// ... call execute_tool_calls(...)
		// ... assert: outcome.steering.is_some()
		// ... assert: outcome.results[0].is_some() (first tool ran)
		// ... assert: outcome.results[1].is_none() (second tool skipped)
	}
```

The exact implementation depends on the test infrastructure. Skeleton provided — flesh out with real `ToolRegistry` setup.

**Step 2: Write integration test for follow-up**

Test that when the follow-up fetcher returns a message after the inner loop, the outer loop continues and processes it. This requires a mock LLM provider, so may need to be a higher-level test or documented as a manual test.

**Step 3: Run all tests**

Run: `cargo test --workspace`
Expected: all pass.

**Step 4: Run clippy and formatting**

Run: `cargo clippy --workspace && cargo fmt --all -- --check`
Expected: zero warnings, formatting clean.

**Step 5: Commit**

```bash
git add crates/rho-agent/src/agent_loop.rs
git commit -m "test: add integration tests for steering and follow-up in dual loop"
```

---

### Task 9: Final validation

**Step 1: Full workspace validation**

Run:

```bash
cargo clippy --workspace
cargo test --workspace
cargo fmt --all -- --check
```

All must pass with zero warnings.

**Step 2: Manual smoke test**

1. `cargo run -p rho` — start the agent
2. Submit a message that triggers multiple tool calls
3. While tools are executing, type a new message and submit
4. Verify: remaining tools show "Skipped due to queued user message." and the agent processes the new message
5. Verify: Ctrl+C still cancels immediately

**Step 3: Final commit (if any fixups needed)**

```bash
git add -A
git commit -m "fix: address issues found during validation"
```
