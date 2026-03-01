# Dual-Loop Agent with Two-Tier Message Queue

**Date:** 2026-03-01
**Status:** Design approved
**Scope:** Add steering messages (mid-turn user input) and follow-up messages (autonomous continuation) to the agent loop using a poll-based two-tier queue.
**Reference:** pi_agent_rust's `MessageQueue` + `MessageFetcher` pattern (`pi-agent-rust:src/agent.rs:88-194,690-968,1582-1806`)

---

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Steering scope | Tool boundaries only | LLM streaming runs to completion. Ctrl+C remains the escape hatch for aborting streams. Most agent time is in tool execution, not streaming. Avoids partial message handling. |
| Follow-up scope | Generic fetcher callback | Trivial outer loop. MaxTokens auto-continue is one consumer in interactive mode. |
| Queue mode | OneAtATime only | Process one queued message per loop iteration. Both references default to this. No concrete use case for All mode yet. |
| Queue sharing | Fetcher callbacks | `Arc<dyn Fn() -> Vec<Message>>` passed into `AgentConfig`. Interactive mode owns the backing `VecDeque` behind `Arc<Mutex<>>`. Clean crate boundaries — `rho-agent` only knows the callback signature. |

---

## Types & Signatures

### New type in `rho-agent`

```rust
/// Synchronous callback that drains queued messages.
/// Returns an empty Vec when no messages are pending.
pub type MessageFetcher = Arc<dyn Fn() -> Vec<Message> + Send + Sync>;
```

Synchronous `Fn`, not async. Fetchers just drain a `Mutex<VecDeque>` — no await needed.

### New fields on `AgentConfig`

```rust
pub struct AgentConfig {
    // ... existing fields ...
    /// Polled at tool execution boundaries. High priority — interrupts tool batches.
    pub steering_fetcher:  Option<MessageFetcher>,
    /// Polled when the inner loop exhausts (no more tool calls or steering).
    pub follow_up_fetcher: Option<MessageFetcher>,
}
```

### Internal return type from tool execution

```rust
struct ToolExecutionOutcome {
    results: Vec<Option<(Arc<String>, bool)>>,
    /// If steering messages arrived during tool execution, remaining tools
    /// were skipped and these messages should be processed next.
    steering: Option<Vec<Message>>,
}
```

No new `AgentOutcome` variants needed — steering doesn't change how the loop terminates.

---

## Agent Loop Structure

The current single loop becomes a nested dual loop:

```
loop {                                          // OUTER: follow-up loop
    drain steering into pending_messages

    loop {                                      // INNER: steering + tool execution
        if pending_messages is empty && no more tool calls:
            break inner

        inject pending_messages into context
        check_should_stop()
        stream LLM

        if tool_calls:
            check_should_stop()
            execute tools (barrier scheduling, with steering polls)
            if steering arrived during tools:
                set pending_messages = steering
                continue inner
            check_should_stop()
            continue inner
        else:
            break inner
    }

    drain follow_up
    if follow_up is empty:
        return outcome
    pending_messages = follow_up
    continue outer
}
```

The LLM streaming loop is unchanged. Tool execution returns a `ToolExecutionOutcome` that may carry steering messages. When steering is found, unexecuted tools get placeholder results.

---

## Steering Poll Points in Tool Execution

Four poll points within the barrier scheduler:

```
emit ToolCallStart for ALL tools upfront

for each tool_call:
    if shared:
        buffer into pending_parallel
    if exclusive:
        >>> POLL 1: before flushing shared batch <<<
        if steering: break

        flush pending_parallel batch

        >>> POLL 2: before expensive exclusive tool <<<
        if steering: break

        race exclusive tool against cancellation

if pending_parallel remaining and no steering:
    >>> POLL 3: before final shared flush <<<
    if steering: skip final flush
    else: flush remaining shared batch

>>> POLL 4: after all execution, during result collection <<<

// Result collection phase:
for each tool_call:
    if result exists:
        emit ToolCallResult, append to history
    else if steering caused skip:
        skip_tool_call("Skipped due to queued user message.")
    else:
        emit aborted result
```

### `skip_tool_call` helper

Synthesizes a `ToolResultMessage` with `content: "Skipped due to queued user message."` and `is_error: true`. Preserves the tool_use → tool_result pairing invariant that the LLM expects. Emits `ToolCallResult` and `ToolResultComplete` events.

### Why 4 poll points

A single poll after all tools would mean steering waits for long-running exclusive tools (e.g. bash). Polling before each exclusive tool gives immediate responsiveness where it matters most. Shared batches are NOT interrupted mid-flight — once `flush_shared_batch` starts `join_all`, it runs to completion. Steering is checked between batches only.

---

## Interactive Mode Integration

### Queue storage

```rust
let steering_queue: Arc<Mutex<VecDeque<Message>>> = Arc::new(Mutex::new(VecDeque::new()));
let follow_up_queue: Arc<Mutex<VecDeque<Message>>> = Arc::new(Mutex::new(VecDeque::new()));
```

### Fetcher construction

```rust
let sq = Arc::clone(&steering_queue);
let steering_fetcher: MessageFetcher = Arc::new(move || {
    let mut q = sq.lock().unwrap_or_else(|e| e.into_inner());
    q.pop_front().into_iter().collect()  // OneAtATime
});
```

Same pattern for `follow_up_fetcher`.

### User input routing

```
if mode == AppMode::Streaming && user submits message:
    push message to steering_queue    // instead of cancel + respawn

if mode == AppMode::Idle && user submits message:
    // existing behavior: spawn_agent with the new message
```

### Cancel (Ctrl+C) unchanged

Fires the `CancellationToken` as before. Steering and cancellation are independent — steering redirects, cancel aborts.

---

## Session Persistence

No new session entry types needed. Steering messages are `Message::User` entries that get appended to history and persisted through the existing event flow:

```
assistant message (with tool_use blocks)
tool_result: "file content..."              // completed before steering
tool_result: "Skipped due to queued..."     // skipped by steering
user message: "actually, focus on X"        // the steering message
assistant message (new response)
```

The interactive mode persists steering messages itself (it has the message since it put it in the queue), mirroring how it currently persists the initial user message before spawning the agent.

---

## Testing Strategy

### Unit tests

- **`skip_tool_call`**: Produces correct `ToolResultMessage` with skip content and `is_error: true`.
- **Fetcher behavior**: Closure correctly drains one message at a time from `VecDeque`.

### Integration tests

- Wire a `MessageFetcher` that returns a steering message after N polls.
- Verify: tools after steering point get skipped results, steering message appears in history, loop continues with new LLM turn.
- Follow-up: verify outer loop re-enters when fetcher returns messages.

### Edge cases

| Scenario | Expected behavior |
|----------|-------------------|
| Steering arrives with no tools executing | Picked up at next `drain_steering` (start of inner loop) |
| Steering arrives during retry backoff | Checked after backoff completes |
| Multiple steering messages queued | OneAtATime processes first; second waits for next boundary |
| Follow-up + steering both present | Steering has priority (inner loop); follow-up checked only when inner loop exhausts |
| Cancel while steering is pending | `CancellationToken` wins, loop exits with `Cancelled` |
| Steering arrives during shared batch | Batch runs to completion; steering checked at next poll point |

---

## Files Affected

| File | Change |
|------|--------|
| `crates/rho-agent/src/agent_loop.rs` | Dual-loop structure, steering polls, `ToolExecutionOutcome`, `skip_tool_call`, `drain_steering`/`drain_follow_up` helpers |
| `crates/rho-agent/src/tools.rs` | Add `MessageFetcher` type alias |
| `crates/rho-agent/src/agent_loop.rs` (`AgentConfig`) | Add `steering_fetcher` and `follow_up_fetcher` fields |
| `crates/rho/src/modes/interactive.rs` | Queue storage, fetcher wiring, input routing change (steer instead of cancel+respawn) |
| `crates/rho/src/modes/interactive.rs` (`spawn_agent`) | Pass fetchers through to `AgentConfig` |
