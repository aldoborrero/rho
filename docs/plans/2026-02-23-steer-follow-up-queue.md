# Agent Steer & Follow-Up Queue — Implementation Plan

> Replaces `docs/steer-queue-plan.md` with concrete, copy-pasteable diffs.
> Lessons from oh-my-pi prototype are folded in.

## Overview

4 phases, all implemented in one pass. Changes touch 4 files.

| File | What changes |
|------|-------------|
| `crates/rho-agent/Cargo.toml` | Add `tokio-util` dep |
| `crates/rho-agent/src/events.rs` | Add `SteerInjected`, `FollowUpInjected` variants |
| `crates/rho-agent/src/agent_loop.rs` | `AgentHandle`, `AgentCommand`, extended `run_agent_loop` |
| `crates/rho/src/modes/interactive.rs` | Generation counter, handle storage, steer/follow-up routing, key mappings |

## Verification

```bash
cargo build -p rho-agent -p rho
cargo test -p rho-agent -p rho
cargo clippy -p rho-agent -p rho
```

---

## Step 1: `crates/rho-agent/Cargo.toml`

Add `tokio-util` to `[dependencies]`:

```toml
tokio-util = "0.7"
```

Place it after `tokio-stream = "0.1"`.

---

## Step 2: `crates/rho-agent/src/events.rs`

Add two new variants to `AgentEvent`, before `Done`:

```rust
/// A steer message was injected mid-turn (for session persistence).
SteerInjected(String),
/// A follow-up message was injected after the agent's natural stop (for session persistence).
FollowUpInjected(String),
```

No changes to `AgentOutcome` — `Cancelled` already exists.

---

## Step 3: `crates/rho-agent/src/agent_loop.rs`

This is the largest change. Replace the entire file with the structure below. Key differences from current code:

### 3a. New imports

Add at top:

```rust
use std::collections::VecDeque;
use tokio_util::sync::CancellationToken;
```

Add `UserMessage` to the `crate::types` import:

```rust
use crate::types::{ContentBlock, Message, ToolResultMessage, UserMessage, Usage};
```

### 3b. New types (add before `ThinkingLevel`)

```rust
/// Commands sent to a running agent loop.
pub enum AgentCommand {
    /// Inject a user message mid-turn, skipping remaining tool calls.
    Steer(String),
    /// Queue a message for after the agent finishes naturally.
    FollowUp(String),
}

/// Handle for a running agent loop.
#[derive(Clone)]
pub struct AgentHandle {
    cancel: CancellationToken,
    cmd_tx: mpsc::Sender<AgentCommand>,
}

impl AgentHandle {
    /// Create a new handle, returning the handle itself plus the token and
    /// receiver that should be passed to `run_agent_loop`.
    pub fn new() -> (Self, CancellationToken, mpsc::Receiver<AgentCommand>) {
        let cancel = CancellationToken::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let token = cancel.clone();
        (Self { cancel, cmd_tx }, token, cmd_rx)
    }

    /// Cancel the running agent loop.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Whether the agent loop has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Try to send a steer command (non-blocking).
    pub fn try_steer(&self, text: String) -> Result<(), mpsc::error::TrySendError<AgentCommand>> {
        self.cmd_tx.try_send(AgentCommand::Steer(text))
    }

    /// Try to send a follow-up command (non-blocking).
    pub fn try_follow_up(
        &self,
        text: String,
    ) -> Result<(), mpsc::error::TrySendError<AgentCommand>> {
        self.cmd_tx.try_send(AgentCommand::FollowUp(text))
    }
}
```

### 3c. Extended `run_agent_loop` signature

Add two new parameters after `event_tx`:

```rust
pub async fn run_agent_loop(
    model: &rho_ai::Model,
    messages: &mut Vec<Message>,
    tools: &ToolRegistry,
    config: AgentConfig,
    event_tx: mpsc::Sender<AgentEvent>,
    abort: Option<CancellationToken>,         // NEW
    mut cmd_rx: mpsc::Receiver<AgentCommand>, // NEW
) -> AgentOutcome {
```

### 3d. New local state (top of function body)

Add after `cumulative_usage`:

```rust
let mut pending_follow_ups: VecDeque<String> = VecDeque::new();
```

### 3e. Cancellation checkpoint 1 (top of outer loop)

Add as the first thing inside `loop {`, before `turn += 1`:

```rust
// Checkpoint 1: check cancellation before streaming.
if abort.as_ref().is_some_and(|t| t.is_cancelled()) {
    return emit_done(AgentOutcome::Cancelled, &event_tx).await;
}
```

### 3f. Pass abort token to stream options (checkpoint 2)

In the `StreamOptions` construction, add:

```rust
let options = rho_ai::types::StreamOptions {
    max_tokens: Some(max_tokens_for(config.thinking, config.max_tokens)),
    reasoning: thinking_to_reasoning(config.thinking),
    retry: config.retry.clone(),
    abort: abort.clone(),  // NEW — checkpoint 2
    ..Default::default()
};
```

### 3g. Post-stream cancellation check

After the `while let Some(event) = event_stream.next().await` loop ends, before the retry check, add:

```rust
// If stream was cancelled by the abort token, return Cancelled.
if abort.as_ref().is_some_and(|t| t.is_cancelled()) {
    return emit_done(AgentOutcome::Cancelled, &event_tx).await;
}
```

### 3h. Replace tool execution block

The current code has:

```rust
if has_tool_calls {
    for block in &message.content {
        if let ContentBlock::ToolUse { id, name, input } = block {
            // ... execute tool ...
        }
    }
    continue;
}
```

Replace with the following. This collects tool_use blocks first, then iterates with steer/cancel checks before each execution:

```rust
// Collect tool_use blocks for processing.
let tool_uses: Vec<_> = message
    .content
    .iter()
    .filter_map(|b| {
        if let ContentBlock::ToolUse { id, name, input } = b {
            Some((id.clone(), name.clone(), input.clone()))
        } else {
            None
        }
    })
    .collect();

// Append assistant message to context
messages.push(Message::Assistant(message.clone()));

if !tool_uses.is_empty() {
    let mut steer_text: Option<String> = None;

    for (i, (id, name, input)) in tool_uses.iter().enumerate() {
        // Drain commands before each tool execution.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                AgentCommand::Steer(text) => {
                    steer_text = Some(text);
                }
                AgentCommand::FollowUp(text) => {
                    pending_follow_ups.push_back(text);
                }
            }
        }

        // If we received a steer, skip this and all remaining tools.
        if let Some(ref text) = steer_text {
            for (skip_id, skip_name, _) in &tool_uses[i..] {
                let _ = event_tx
                    .send(AgentEvent::ToolCallStart {
                        id: skip_id.clone(),
                        name: skip_name.clone(),
                    })
                    .await;

                let content = "Skipped due to new user message.".to_owned();
                let _ = event_tx
                    .send(AgentEvent::ToolCallResult {
                        id: skip_id.clone(),
                        is_error: true,
                        content: content.clone(),
                    })
                    .await;
                let _ = event_tx
                    .send(AgentEvent::ToolResultComplete {
                        tool_use_id: skip_id.clone(),
                        content: content.clone(),
                        is_error: true,
                    })
                    .await;

                messages.push(Message::ToolResult(ToolResultMessage {
                    tool_use_id: skip_id.clone(),
                    content,
                    is_error: true,
                }));
            }

            // Inject the steer user message.
            let _ = event_tx
                .send(AgentEvent::SteerInjected(text.clone()))
                .await;
            messages.push(Message::User(UserMessage {
                content: text.clone(),
            }));
            break;
        }

        // Checkpoint 3: check cancellation before each tool execution.
        if abort.as_ref().is_some_and(|t| t.is_cancelled()) {
            for (cancel_id, cancel_name, _) in &tool_uses[i..] {
                let _ = event_tx
                    .send(AgentEvent::ToolCallStart {
                        id: cancel_id.clone(),
                        name: cancel_name.clone(),
                    })
                    .await;

                let content = "Cancelled.".to_owned();
                let _ = event_tx
                    .send(AgentEvent::ToolCallResult {
                        id: cancel_id.clone(),
                        is_error: true,
                        content: content.clone(),
                    })
                    .await;
                let _ = event_tx
                    .send(AgentEvent::ToolResultComplete {
                        tool_use_id: cancel_id.clone(),
                        content: content.clone(),
                        is_error: true,
                    })
                    .await;

                messages.push(Message::ToolResult(ToolResultMessage {
                    tool_use_id: cancel_id.clone(),
                    content,
                    is_error: true,
                }));
            }
            return emit_done(AgentOutcome::Cancelled, &event_tx).await;
        }

        // Execute the tool normally.
        let _ = event_tx
            .send(AgentEvent::ToolCallStart {
                id: id.clone(),
                name: name.clone(),
            })
            .await;

        let tool_result =
            tools.execute(name, input.clone(), &config.cwd).await;

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

        let _ = event_tx
            .send(AgentEvent::ToolResultComplete {
                tool_use_id: id.clone(),
                content: content.clone(),
                is_error,
            })
            .await;

        messages.push(Message::ToolResult(ToolResultMessage {
            tool_use_id: id.clone(),
            content,
            is_error,
        }));
    }

    // Loop back to send tool results (or steer message) to LLM
    continue;
}
```

**Important:** The old code had `messages.push(Message::Assistant(message.clone()))` and `has_tool_calls` check separately. The new code collects tool_uses first, pushes the assistant message, then branches. Remove the old `has_tool_calls` variable and the old `messages.push(Message::Assistant(...))` line that preceded it.

### 3i. Follow-up drain at exit point

Replace the current "no tool calls" terminal condition block with:

```rust
// No tool calls — drain follow-ups from channel + local accumulator.
while let Ok(cmd) = cmd_rx.try_recv() {
    match cmd {
        AgentCommand::Steer(text) | AgentCommand::FollowUp(text) => {
            pending_follow_ups.push_back(text);
        }
    }
}

if let Some(text) = pending_follow_ups.pop_front() {
    let _ = event_tx
        .send(AgentEvent::FollowUpInjected(text.clone()))
        .await;
    messages.push(Message::User(UserMessage { content: text }));
    continue; // Loop back to LLM
}

// Truly done — check terminal conditions.
let outcome = match message.stop_reason.as_ref() {
    Some(crate::types::StopReason::MaxTokens) => AgentOutcome::MaxTokens {
        usage: cumulative_usage,
    },
    _ => AgentOutcome::Stop {
        usage: cumulative_usage,
    },
};
return emit_done(outcome, &event_tx).await;
```

### 3j. Tests

Add to the existing `#[cfg(test)] mod tests` block:

```rust
use crate::events::AgentOutcome;

#[test]
fn test_agent_handle_cancel() {
    let (handle, token, _rx) = AgentHandle::new();
    assert!(!handle.is_cancelled());
    assert!(!token.is_cancelled());
    handle.cancel();
    assert!(handle.is_cancelled());
    assert!(token.is_cancelled());
}

#[test]
fn test_agent_handle_clone_shares_state() {
    let (handle, token, _rx) = AgentHandle::new();
    let handle2 = handle.clone();
    handle2.cancel();
    assert!(handle.is_cancelled());
    assert!(token.is_cancelled());
}

#[tokio::test]
async fn test_agent_handle_try_steer() {
    let (handle, _token, mut rx) = AgentHandle::new();
    handle.try_steer("fix the imports".to_owned()).unwrap();
    match rx.recv().await.unwrap() {
        AgentCommand::Steer(text) => assert_eq!(text, "fix the imports"),
        _ => panic!("expected Steer command"),
    }
}

#[tokio::test]
async fn test_agent_handle_try_follow_up() {
    let (handle, _token, mut rx) = AgentHandle::new();
    handle.try_follow_up("then run tests".to_owned()).unwrap();
    match rx.recv().await.unwrap() {
        AgentCommand::FollowUp(text) => assert_eq!(text, "then run tests"),
        _ => panic!("expected FollowUp command"),
    }
}

#[tokio::test]
async fn test_pre_cancelled_token_returns_cancelled() {
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
    let token = CancellationToken::new();
    token.cancel(); // Pre-cancel

    let (_cmd_tx, cmd_rx) = mpsc::channel::<AgentCommand>(8);
    let tools = crate::registry::ToolRegistry::new();
    let config = AgentConfig {
        system_prompt: String::new(),
        max_tokens: 8192,
        thinking: ThinkingLevel::Off,
        retry: rho_ai::RetryConfig::default(),
        cwd: std::env::current_dir().unwrap_or_default(),
    };

    let model = rho_ai::models::Model {
        id: "test".into(),
        name: "test".into(),
        provider: "test".into(),
        api: rho_ai::models::Api::AnthropicMessages,
        base_url: "https://example.com".into(),
        reasoning: false,
        supports_images: false,
        context_window: 200_000,
        max_tokens: 8192,
        cost: rho_ai::models::ModelCost::default(),
    };

    let mut messages = vec![Message::User(UserMessage {
        content: "hello".to_owned(),
    })];

    let outcome = run_agent_loop(
        &model, &mut messages, &tools, config, event_tx,
        Some(token), cmd_rx,
    )
    .await;

    assert!(matches!(outcome, AgentOutcome::Cancelled));
    let event = event_rx.recv().await.unwrap();
    assert!(matches!(event, AgentEvent::Done(AgentOutcome::Cancelled)));
}

#[tokio::test]
async fn test_none_abort_does_not_cancel() {
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
    let (_cmd_tx, cmd_rx) = mpsc::channel::<AgentCommand>(8);
    let tools = crate::registry::ToolRegistry::new();
    let config = AgentConfig {
        system_prompt: String::new(),
        max_tokens: 8192,
        thinking: ThinkingLevel::Off,
        retry: rho_ai::RetryConfig { enabled: false, ..Default::default() },
        cwd: std::env::current_dir().unwrap_or_default(),
    };

    let model = rho_ai::models::Model {
        id: "test".into(),
        name: "test".into(),
        provider: "test".into(),
        api: rho_ai::models::Api::AnthropicMessages,
        base_url: "https://invalid.example.com".into(),
        reasoning: false,
        supports_images: false,
        context_window: 200_000,
        max_tokens: 8192,
        cost: rho_ai::models::ModelCost::default(),
    };

    let mut messages = vec![Message::User(UserMessage {
        content: "hello".to_owned(),
    })];

    let outcome = run_agent_loop(
        &model, &mut messages, &tools, config, event_tx,
        None, cmd_rx,
    )
    .await;

    assert!(!matches!(outcome, AgentOutcome::Cancelled));
    let mut found_done = false;
    while let Ok(event) = event_rx.try_recv() {
        if matches!(event, AgentEvent::Done(_)) { found_done = true; }
    }
    assert!(found_done);
}
```

---

## Step 4: `crates/rho/src/modes/interactive.rs`

### 4a. Import `AgentHandle`

Change:

```rust
use rho_agent::agent_loop::{AgentConfig, ThinkingLevel};
```

To:

```rust
use rho_agent::agent_loop::{AgentConfig, AgentHandle, ThinkingLevel};
```

### 4b. Update `AppEvent`

**Important:** `gen` is a reserved keyword in Rust 2024. Use `generation`.

Change:

```rust
pub enum AppEvent {
    Terminal(rho_tui::TerminalEvent),
    EditorSubmit(String),
    Agent(AgentEvent),
}
```

To:

```rust
pub enum AppEvent {
    Terminal(rho_tui::TerminalEvent),
    EditorSubmit(String),
    Agent { generation: u64, event: AgentEvent },
}
```

### 4c. Update `spawn_agent`

Replace the entire function:

```rust
/// Spawn the autonomous agent loop in a background task.
///
/// Returns an `AgentHandle` for cancellation and steering. Events are
/// tagged with `generation` so the event loop can drop stale events.
fn spawn_agent(
    model: &rho_ai::Model,
    messages: &[Message],
    tools: &ToolRegistry,
    system_prompt: &str,
    cli: &Cli,
    tx: &tokio::sync::mpsc::Sender<AppEvent>,
    generation: u64,
) -> AgentHandle {
    let (handle, abort_token, cmd_rx) = AgentHandle::new();
    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

    let forward_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            if forward_tx
                .send(AppEvent::Agent { generation, event })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let agent_model = model.clone();
    let agent_tools = tools.clone();
    let agent_config = AgentConfig {
        system_prompt: system_prompt.to_owned(),
        max_tokens: 8192,
        thinking: parse_thinking(&cli.thinking),
        retry: rho_ai::RetryConfig::default(),
        cwd: std::env::current_dir().unwrap_or_default(),
    };
    let mut agent_messages = messages.to_vec();
    tokio::spawn(async move {
        let _outcome = rho_agent::agent_loop::run_agent_loop(
            &agent_model,
            &mut agent_messages,
            &agent_tools,
            agent_config,
            agent_tx,
            Some(abort_token),
            cmd_rx,
        )
        .await;
    });

    handle
}
```

### 4d. Replace mode tracking with agent handle

Replace:

```rust
// Track application mode.
let mut mode = AppMode::Idle;
```

With:

```rust
// Track agent handle and generation counter.
// `agent_handle.is_some()` is the ground truth for "agent is running".
let mut agent_handle: Option<AgentHandle> = None;
let mut agent_generation: u64 = 0;
```

The `AppMode` enum is **no longer used** for Idle/Streaming tracking. Remove the `use crate::modes::state::AppMode;` import if it becomes unused.

### 4e. Update initial message spawn

Replace:

```rust
mode = AppMode::Streaming;
app.chat.borrow_mut().start_streaming();
spawn_agent(
    &model, session.messages(), &tools, &system_prompt, cli, &tx,
);
```

With:

```rust
agent_generation += 1;
app.chat.borrow_mut().start_streaming();
agent_handle = Some(spawn_agent(
    &model, session.messages(), &tools, &system_prompt, cli, &tx,
    agent_generation,
));
```

### 4f. Update Ctrl+C handler

Replace:

```rust
if data == "\x03" {
    if matches!(mode, AppMode::Streaming) {
        mode = AppMode::Idle;
        app.chat.borrow_mut().finish_streaming();
    } else {
        break;
    }
}
```

With:

```rust
if data == "\x03" {
    if let Some(ref h) = agent_handle {
        h.cancel();
    } else {
        break;
    }
}
```

Key: do NOT call `finish_streaming()` or clear `agent_handle` here. Wait for `Done(Cancelled)` to arrive naturally.

### 4g. Add Alt+Enter handler for follow-up

After the Ctrl+O handler (`else if data == "\x0f"`), add:

```rust
// Alt+Enter while agent is running: queue follow-up.
else if agent_handle.is_some() && data == "\x1b\r" {
    let text = app.editor.borrow().get_text();
    let text = text.trim().to_owned();
    if !text.is_empty() {
        app.editor.borrow_mut().set_text("");
        if let Some(ref h) = agent_handle {
            let _ = h.try_follow_up(text);
        }
    }
}
```

The `else` branch forwarding input to `app.tui.handle_input(data)` should remain unconditional (no `!is_streaming` guard) — input is always forwarded so the user can type steer/follow-up messages.

### 4h. Update `EditorSubmit` handler

The `EditorSubmit` handler currently uses `route_input` to classify input. The critical change: **block slash/bang commands while streaming, route user messages as steers.**

In the `EditorSubmit` arm, after `app.editor.borrow_mut().add_to_history(&text);`, add this early return before `match route_input(&text)`:

```rust
// While agent is running, all input is treated as a steer.
// Slash/bang commands are blocked to prevent unsafe concurrent
// operations (e.g., /new clearing the session mid-stream).
if agent_handle.is_some() {
    match route_input(&text) {
        InputAction::SlashCommand { .. } | InputAction::UnknownCommand(_) | InputAction::BangCommand(_) => {
            show_chat_message(
                &mut app,
                "Cannot run commands while streaming. Type a message to steer the agent.",
            );
        }
        InputAction::UserMessage(msg) => {
            if let Some(ref h) = agent_handle {
                let _ = h.try_steer(msg.to_owned());
            }
        }
        InputAction::Empty => {}
    }
    app.tui.request_render();
    app.tui.render(&mut terminal)?;
    continue;
}
```

In the existing `InputAction::UserMessage` arm, remove the old "if streaming, interrupt" logic:

```rust
// OLD — remove this:
if matches!(mode, AppMode::Streaming) {
    mode = AppMode::Idle;
    app.chat.borrow_mut().finish_streaming();
}
```

And update the spawn:

```rust
InputAction::UserMessage(text) => {
    let user_msg = Message::User(UserMessage {
        content: text.to_owned(),
    });
    app.chat.borrow_mut().add_message(user_msg.clone());
    session.append(user_msg).await?;

    agent_generation += 1;
    app.chat.borrow_mut().start_streaming();
    agent_handle = Some(spawn_agent(
        &model, session.messages(), &tools, &system_prompt, cli, &tx,
        agent_generation,
    ));
}
```

### 4i. Update agent event handler

Replace `Some(AppEvent::Agent(agent_event))` with generation-guarded version:

```rust
Some(AppEvent::Agent { generation, event: agent_event }) => {
    // Drop stale events from old agent runs.
    if generation != agent_generation {
        app.tui.request_render();
        app.tui.render(&mut terminal)?;
        continue;
    }

    match agent_event {
        AgentEvent::TurnStart { .. } => {}
        AgentEvent::TextDelta(text) => {
            app.chat.borrow_mut().append_text(&text);
        }
        AgentEvent::ThinkingDelta(text) => {
            app.chat.borrow_mut().append_thinking(&text);
        }
        AgentEvent::ToolCallStart { .. } => {}
        AgentEvent::ToolCallResult { .. } => {}
        AgentEvent::MessageComplete(message) => {
            if let Some(ref usage) = message.usage {
                app.status
                    .set_usage(usage.input_tokens, usage.output_tokens);
                app.update_status_border(terminal.columns());
            }
            session.append(Message::Assistant(message.clone())).await?;
            // Don't call finish_streaming() here — tools may still
            // be executing. Streaming ends on Done.
            app.chat.borrow_mut().add_message(Message::Assistant(message));
        }
        AgentEvent::ToolResultComplete {
            tool_use_id,
            content,
            is_error,
        } => {
            let tool_msg = Message::ToolResult(ToolResultMessage {
                tool_use_id,
                content,
                is_error,
            });
            app.chat.borrow_mut().add_message(tool_msg.clone());
            session.append(tool_msg).await?;
            app.chat.borrow_mut().start_streaming();
        }
        AgentEvent::SteerInjected(text) => {
            let user_msg = Message::User(UserMessage { content: text });
            app.chat.borrow_mut().add_message(user_msg.clone());
            session.append(user_msg).await?;
            app.chat.borrow_mut().start_streaming();
        }
        AgentEvent::FollowUpInjected(text) => {
            let user_msg = Message::User(UserMessage { content: text });
            app.chat.borrow_mut().add_message(user_msg.clone());
            session.append(user_msg).await?;
            app.chat.borrow_mut().start_streaming();
        }
        AgentEvent::RetryScheduled {
            attempt,
            delay_ms,
            error,
        } => {
            show_chat_message(
                &mut app,
                &format!("Retrying (attempt {attempt}) in {delay_ms}ms: {error}"),
            );
        }
        AgentEvent::Done(outcome) => {
            agent_handle = None;
            app.chat.borrow_mut().finish_streaming();
            match outcome {
                AgentOutcome::MaxTokens { .. } => {
                    show_chat_message(
                        &mut app,
                        "Warning: response truncated (max tokens reached).",
                    );
                }
                AgentOutcome::Failed { error } => {
                    show_chat_message(&mut app, &format!("Error: {error}"));
                }
                AgentOutcome::Cancelled => {
                    show_chat_message(&mut app, "Cancelled.");
                }
                AgentOutcome::Stop { .. } => {}
            }
        }
    }
}
```

Key differences from old code:
- Generation guard at top
- `TextDelta`/`ThinkingDelta` no longer guarded by `is_streaming` (generation counter handles stale events)
- `MessageComplete` does NOT call `finish_streaming()` (tools may still run)
- `SteerInjected`/`FollowUpInjected` persist user message + restart streaming
- `Done` clears `agent_handle` (not `mode`), shows Cancelled message
- No `mode = AppMode::Idle` anywhere

### 4j. Update `crossterm_key_to_string` for Alt+Enter and Alt+Up

Replace:

```rust
KeyCode::Enter => "\r".to_owned(),
```

With:

```rust
KeyCode::Enter => {
    if key.modifiers.contains(KeyModifiers::ALT) {
        "\x1b\r".to_owned()
    } else {
        "\r".to_owned()
    }
}
```

Replace:

```rust
KeyCode::Up => "\x1b[A".to_owned(),
```

With:

```rust
KeyCode::Up => {
    if key.modifiers.contains(KeyModifiers::ALT) {
        "\x1b[1;3A".to_owned()
    } else {
        "\x1b[A".to_owned()
    }
}
```

Add tests:

```rust
#[test]
fn test_crossterm_key_to_string_alt_enter() {
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::ALT,
    );
    assert_eq!(crossterm_key_to_string(&key), "\x1b\r");
}

#[test]
fn test_crossterm_key_to_string_alt_up() {
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyModifiers::ALT,
    );
    assert_eq!(crossterm_key_to_string(&key), "\x1b[1;3A");
}
```

---

## Gotchas from prototype

1. **`gen` is reserved in Rust 2024** — use `generation` everywhere
2. **Don't call `finish_streaming()` in `MessageComplete`** — tools haven't run yet, it causes visual flicker
3. **Don't track `is_streaming: bool` separately** — `agent_handle.is_some()` is the ground truth; a separate boolean creates dual-tracking bugs
4. **Block slash/bang commands during streaming** — otherwise `/new` can clear the session while the agent writes to it
5. **Channel buffer 32, not 8** — with `try_send`, a small buffer means silent drops under rapid input
6. **`AppMode` becomes unused for Idle/Streaming** — if other variants aren't used either, you can remove the import. Keep the enum in `state.rs` for future modes.

## Future work (not in this plan)

- Pending message display (`Vec<PendingMessage>` visual tracking below editor)
- Alt+Up dequeue (requires `PopLast` command variant in `AgentCommand`)
- Input mode indicator in status bar
- Integration tests for steer/follow-up (requires mocking `rho_ai::stream()`)
