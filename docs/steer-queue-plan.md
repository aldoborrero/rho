# Agent Steer & Follow-Up Queue

## Context

The interactive loop currently has no way to communicate with a running agent. `spawn_agent()` is fire-and-forget — it spawns a tokio task and returns nothing. Ctrl+C just flips `mode = Idle` and ignores subsequent agent events. Submitting a message while streaming does the same crude interrupt.

The TS reference implementation has a proper system: **steer** (inject a user message mid-turn, skipping remaining tool calls) and **followUp** (queue a message for after the agent finishes naturally). This plan ports that to Rust in 4 phases, each independently shippable.

## Architecture Decision: Handle Pattern (not Single Instance)

The TS uses a single `Agent` instance that owns messages and gets abort()'d/reused. This doesn't translate well to Rust because:

1. **Message ownership across task boundaries** — `run_agent_loop` takes `&mut Vec<Message>` on a spawned task. Sharing requires `Arc<Mutex<>>` or moving messages back via `JoinHandle`, both awkward.
2. **Non-Send tools** — `ToolRegistry` contains non-Send types. A struct holding both tools and a task handle fights `Send` bounds.
3. **No benefit over handles** — The "one loop at a time" invariant is already enforced by `AppMode::Streaming` + cancellation.

**Our approach:** Pure function (`run_agent_loop`) + lightweight `AgentHandle` (cancel token + command channel). The Rust-idiomatic equivalent that gives the same guarantees without fighting the borrow checker.

**Stale event filtering:** A `u64` generation counter on `AppEvent::Agent` unconditionally drops events from old agent runs. This handles the Ctrl+C → immediate re-prompt race (where an old agent's `Done(Cancelled)` could corrupt the new agent's state). Each `spawn_agent` increments the counter; events with a mismatched generation are skipped at the top of the `Agent` match arm.

## Existing Infrastructure

- `StreamOptions.abort: Option<CancellationToken>` — already wired into all 3 providers (`rho-ai`)
- `AgentOutcome::Cancelled` — variant exists in `rho-agent` but is never returned
- `tokio-util = "0.7"` — already a dep of `rho-ai` and `rho-tools`, not yet of `rho-agent`
- `AppMode` enum — already in place from the state machine refactor

## Known Limitations

- **Steering only works between tool calls, not during.** If a bash tool runs for 30s, the steer won't be picked up until it finishes. The `rho-tools` `CancelToken` system isn't wired to the agent-level `CancellationToken`. Same behavior as the TS `interruptMode: "immediate"`. Bridging the two cancellation systems is a separate effort.

---

## Phase 1: Abort Support + Generation Counter

**Goal:** Ctrl+C actually cancels the running agent. `spawn_agent` returns an `AgentHandle`. A `u64` generation counter on `AppEvent::Agent` drops stale events from old agent runs.

### Files

| File | Change |
|------|--------|
| `crates/rho-agent/Cargo.toml` | Add `tokio-util = "0.7"` |
| `crates/rho-agent/src/agent_loop.rs` | Add `AgentHandle`, `AgentCommand` (empty), new params `abort` + `cmd_rx` |
| `crates/rho/src/modes/interactive.rs` | `spawn_agent` returns `AgentHandle`, store `agent_generation: u64`, Ctrl+C calls `handle.cancel()`, generation guard on all agent events |

### New types in `agent_loop.rs`

```rust
use tokio_util::sync::CancellationToken;

/// Commands sent to a running agent loop (extended in later phases).
pub enum AgentCommand {}

/// Handle for a running agent loop.
#[derive(Clone)]
pub struct AgentHandle {
    cancel: CancellationToken,
    cmd_tx: mpsc::Sender<AgentCommand>,
}

impl AgentHandle {
    pub fn new() -> (Self, CancellationToken, mpsc::Receiver<AgentCommand>) {
        let cancel = CancellationToken::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        let token = cancel.clone();
        (Self { cancel, cmd_tx }, token, cmd_rx)
    }

    pub fn cancel(&self) { self.cancel.cancel(); }
    pub fn is_cancelled(&self) -> bool { self.cancel.is_cancelled() }
}
```

Note: `AgentCommand` starts as an empty enum and `cmd_rx` is passed to `run_agent_loop` but ignored. This avoids breaking the API when Phase 2 adds `Steer`.

### Signature change for `run_agent_loop`

```rust
pub async fn run_agent_loop(
    model: &rho_ai::Model,
    messages: &mut Vec<Message>,
    tools: &ToolRegistry,
    config: AgentConfig,
    event_tx: mpsc::Sender<AgentEvent>,
    abort: Option<CancellationToken>,         // NEW
    mut cmd_rx: mpsc::Receiver<AgentCommand>, // NEW (ignored in Phase 1)
) -> AgentOutcome {
```

### Cancellation check points (3 locations)

1. **Top of outer loop** (before streaming): early exit if cancelled
2. **`StreamOptions.abort`**: pass `abort.clone()` — providers already handle it
3. **Before each tool execution**: exit with `Cancelled`, filling remaining tool_use IDs with "cancelled" results to keep message history valid

### Generation counter

`AppEvent::Agent(AgentEvent)` becomes `AppEvent::Agent { gen: u64, event: AgentEvent }`. The forwarding task in `spawn_agent` tags events with the current generation:

```rust
let gen = agent_generation;
tokio::spawn(async move {
    while let Some(event) = agent_rx.recv().await {
        if forward_tx.send(AppEvent::Agent { gen, event }).await.is_err() {
            break;
        }
    }
});
```

The interactive loop stores `agent_generation: u64` (incremented on each spawn). At the top of the `Agent` match arm:

```rust
Some(AppEvent::Agent { gen, event }) => {
    if gen != agent_generation { continue; } // Drop stale events
    // ... process event ...
}
```

### Interactive loop changes

- `spawn_agent` takes the current generation, returns `AgentHandle`
- Store `agent_handle: Option<AgentHandle>` and `agent_generation: u64`
- On spawn: increment `agent_generation`, pass to `spawn_agent`
- Ctrl+C: call `handle.cancel()` — do NOT immediately set `mode = Idle`. Wait for `AgentEvent::Done(Cancelled)` to arrive naturally. Generation guard ensures only the current agent's `Done` is processed.
- On `AgentEvent::Done`: clear `agent_handle = None`, set `mode = Idle`

---

## Phase 2: Steering (Mid-Turn Interrupt)

**Goal:** Submitting a message while streaming injects it into the running agent's conversation. Remaining tool calls are skipped.

### Files

| File | Change |
|------|--------|
| `crates/rho-agent/src/agent_loop.rs` | Add `Steer(String)` to `AgentCommand`, add `steer()`/`try_steer()` to `AgentHandle`, steering logic in tool loop |
| `crates/rho-agent/src/events.rs` | Add `SteerInjected(String)` variant |
| `crates/rho/src/modes/interactive.rs` | On submit while streaming: `handle.try_steer(text)`. Handle `SteerInjected` event for persistence + display. |

### Agent loop tool execution changes

Replace the sequential `for block in &message.content` with logic that checks `cmd_rx.try_recv()` after each tool call:

```
for each tool_use block:
    drain cmd_rx:
        Steer(text) → skip this + remaining tools with "Skipped" results, inject user message, continue outer loop
        FollowUp(text) → push to local pending_follow_ups VecDeque (Phase 3)
    if abort cancelled → fill remaining with "cancelled", return Cancelled
    execute tool normally
```

Skipped tools get `ToolResultMessage { content: "Skipped due to new user message.", is_error: true }` — and emit `ToolCallResult` + `ToolResultComplete` events so the UI and session stay in sync.

After skipping, inject the steer text as `Message::User(UserMessage { content: text })` and emit `AgentEvent::SteerInjected(text)`.

### Interactive loop changes

```rust
InputAction::UserMessage(text) => {
    if matches!(mode, AppMode::Streaming) {
        if let Some(ref h) = agent_handle {
            let _ = h.try_steer(text.to_owned());
        }
        // Don't touch mode — agent is still streaming.
        // The steer text will appear via SteerInjected event.
    } else {
        // Normal path: spawn new agent
    }
}
```

Handle `SteerInjected`:
```rust
AgentEvent::SteerInjected(text) => {
    let user_msg = Message::User(UserMessage { content: text });
    app.chat.borrow_mut().add_message(user_msg.clone());
    session.append(user_msg).await?;
}
```

---

## Phase 3: Follow-Up Queue

**Goal:** Messages queued for after the agent finishes naturally. Checked at the "no tool calls -> would stop" exit point.

### Files

| File | Change |
|------|--------|
| `crates/rho-agent/src/agent_loop.rs` | Add `FollowUp(String)` to `AgentCommand`, follow-up drain at exit point, local `VecDeque<String>` accumulator |
| `crates/rho-agent/src/events.rs` | Add `FollowUpInjected(String)` variant |
| `crates/rho-agent/src/agent_loop.rs` | Add `follow_up()`/`try_follow_up()` to `AgentHandle` |
| `crates/rho/src/modes/interactive.rs` | Handle `FollowUpInjected` event |

### Agent loop exit point change

At the "no tool calls" section (currently lines 219-231), before returning:

```rust
// Drain follow-ups from channel + local accumulator
while let Ok(cmd) = cmd_rx.try_recv() {
    match cmd {
        AgentCommand::Steer(text) | AgentCommand::FollowUp(text) => {
            pending_follow_ups.push_back(text);
        }
    }
}

if let Some(text) = pending_follow_ups.pop_front() {
    event_tx.send(AgentEvent::FollowUpInjected(text.clone())).await;
    messages.push(Message::User(UserMessage { content: text }));
    continue; // Loop back to LLM
}

// Truly done — return outcome
```

The `pending_follow_ups: VecDeque<String>` lives at the top of `run_agent_loop`. The steering check in the tool loop also drains `FollowUp` commands into this queue so they aren't lost.

### Interactive loop

For now, follow-up is not exposed via keybinding — any message submitted while streaming is a steer (Phase 2). Follow-up is available programmatically for commands like `/compact`. The UI toggle comes in Phase 4.

Handle `FollowUpInjected` the same way as `SteerInjected`.

---

## Phase 4: UI Affordances

**Goal:** Show pending queued messages, keybinding to switch steer/follow-up, dequeue.

### Files

| File | Change |
|------|--------|
| `crates/rho/src/modes/interactive.rs` | Track `pending_display: Vec<PendingMessage>`, keybinding for mode toggle + dequeue |
| `crates/rho/src/tui/status.rs` | Show input mode during streaming |

### Pending message display

Track a local `Vec<PendingMessage>` in the event loop. Add on `try_steer`/`try_follow_up`, remove on `SteerInjected`/`FollowUpInjected`. Render below the editor as:

```
  ↳ Steer: fix the import order
  ↳ Follow-up: then run the tests
  (Alt+Up to edit)
```

### Keybindings

- **Enter** while streaming: steer (default, from Phase 2)
- **Alt+Enter** while streaming: follow-up (queue for later)
- **Alt+Up** while streaming: pop last queued message back into editor (dequeue)

### Dequeue mechanics

Since messages are sent through the channel and the agent may have already consumed them, dequeue only works for messages not yet consumed. The `AgentHandle` needs `pop_last_steer()` / `pop_last_follow_up()` methods, which requires the agent loop to support a `PopLast` command variant. This is a small extension to the `AgentCommand` enum.

---

## Verification (all phases)

```bash
cargo build -p rho-agent -p rho
cargo test -p rho-agent -p rho
cargo clippy -p rho-agent -p rho
```

### Test strategy per phase

- **Phase 1:** Unit test — pre-cancelled token -> `run_agent_loop` returns `Cancelled` immediately. Unit test — `None` abort behaves identically to current code. Unit test — generation counter: events with stale generation are dropped before processing.
- **Phase 2:** Unit test — mock tool that sleeps, send `Steer` before second tool, assert second tool's result is "Skipped" and user message appears in `messages`.
- **Phase 3:** Unit test — agent with no tool calls + `FollowUp` in channel -> turn count increments, follow-up injected. Multiple follow-ups consumed one per turn.
- **Phase 4:** Unit test for pending message rendering. Manual TUI testing.
