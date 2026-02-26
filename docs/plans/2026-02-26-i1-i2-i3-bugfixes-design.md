# Design: Fix I1, I2, I3 — Bang Persistence, Streaming Flash, Compaction Error

Date: 2026-02-26

## Context

Three confirmed bugs in `crates/rho/src/modes/interactive.rs` and related modules:

- **I1**: Bang command output never persisted to session — lost on `--resume`
- **I2**: `MessageComplete` causes visual flash — streaming buffer cleared then re-added
- **I3**: Auto-compaction persistence error silently swallowed

## Reference

Designs are informed by the oh-my-pi TypeScript reference implementation.

---

## I1: Bang Output Persistence

### Problem

When a user runs `!cmd`, the command text is persisted as a `UserMessage` but the
command's stdout/stderr output is only added to the chat display (`app.chat`),
never to `session.append()`. On `--resume`, bang command results are lost.

### Design (matching oh-my-pi)

Oh-my-pi stores bang output as a `BashExecutionMessage` with `role: "bashExecution"`
inside a regular `SessionMessageEntry`. The message contains both the command and
its output. It is NOT stored as a separate entry type.

#### New `Message` variant

Add `Message::BashExecution(BashExecutionMessage)` to the `Message` enum in
`crates/rho-agent/src/types.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashExecutionMessage {
    pub command:              String,
    pub output:               String,
    pub exit_code:            Option<i32>,
    pub cancelled:            bool,
    pub truncated:            bool,
    #[serde(default)]
    pub exclude_from_context: bool,
    pub timestamp:            i64,
}
```

Tagged with `#[serde(rename = "bashExecution")]` on the `Message` enum variant.

#### Support `!!` (exclude from context)

`InputAction::BangCommand` gains a `bool` field for `exclude_from_context`:

```rust
BangCommand { cmd: &'a str, exclude_from_context: bool },
```

`route_input()` detects `!!` prefix and sets the flag accordingly.

#### Persistence flow changes (`interactive.rs`)

1. **Remove** the current `UserMessage` persistence for `!cmd` — the
   `BashExecution` message stores both command and output.
2. **Capture output**: `AppEvent::BangDone` gains fields: `command: String`,
   `output: String`, `exit_code: Option<i32>`, `cancelled: bool`.
   The spawned task collects output and sends it back.
3. **On `BangDone`**: Build a `BashExecutionMessage` and call
   `session.append(Message::BashExecution(...))`.
4. `finish_bang()` returns `Option<(String, String, bool)>` (command, output,
   is_error) so the event loop can access the accumulated output.

#### Context building (`session/context.rs`)

When processing `SessionEntry::Message` containing `Message::BashExecution`:
- If `exclude_from_context` is false: convert to `Message::User(UserMessage)`
  with a formatted text like `$ command\noutput` (matching oh-my-pi's
  `bashExecutionToText`).
- If `exclude_from_context` is true: skip (do not include in LLM context).

#### Resume (`interactive.rs`)

The `for msg in session.messages()` loop gains a match arm for
`Message::BashExecution` that calls `app.chat.add_bang_output(command, output,
is_error)`.

---

## I2: MessageComplete Visual Flash

### Problem

On `AgentEvent::MessageComplete`, two calls happen in sequence:
1. `app.chat.finish_streaming()` — clears `streaming_text` (text disappears)
2. `app.chat.add_message(Message::Assistant(message))` — adds full message (text reappears)

Between these calls, the text is absent. A single render frame shows the flash.

### Design (matching oh-my-pi)

Oh-my-pi updates the same component in-place. On `message_end`, it does a final
`updateContent()` and clears the streaming reference — the component stays in
the container. No remove+re-add cycle.

#### New method: `finish_streaming_with_message()` (`tui/chat.rs`)

```rust
pub fn finish_streaming_with_message(&mut self, message: Message) {
    self.is_streaming = false;
    self.streaming_text.clear();
    self.streaming_thinking.clear();
    self.loader.stop();
    self.tool_executing = None;
    self.items.push(ChatItem::Message(message));
}
```

#### Change in `interactive.rs` at `MessageComplete` handler

Replace:
```rust
app.chat.finish_streaming();
app.chat.add_message(Message::Assistant(message));
```

With:
```rust
app.chat.finish_streaming_with_message(Message::Assistant(message));
```

The streaming buffer and committed message are never both absent in the same
render frame. Existing `finish_streaming()` without a message remains for cancel
paths and `Done` events.

---

## I3: Auto-Compaction Error Swallowed

### Problem

Line 741 uses `let _ = session.append_compaction(...)`. If the disk write fails,
the user sees "Auto-compacted: ..." success message anyway.

### Design (matching oh-my-pi)

Oh-my-pi logs errors to stderr, emits an event with the error message, and the
UI shows a warning. The session continues (not fatal).

#### Change in `interactive.rs` at auto-compaction handler

Replace:
```rust
let _ = session.append_compaction(...);
let msg = result.short_summary...;
show_chat_message(&mut app, &format!("Auto-compacted: {msg}"));
```

With:
```rust
match session.append_compaction(...) {
    Ok(()) => {
        let msg = result.short_summary...;
        show_chat_message(&mut app, &format!("Auto-compacted: {msg}"));
    }
    Err(e) => {
        show_chat_message(
            &mut app,
            &format!("Auto-compaction succeeded but failed to persist: {e}"),
        );
    }
}
```

The compaction result (LLM call) succeeded; only the disk write failed. The
message reflects that distinction.

---

## Files Summary

| File | Changes |
|------|---------|
| `crates/rho-agent/src/types.rs` | Add `BashExecutionMessage` struct and `Message::BashExecution` variant |
| `crates/rho/src/modes/input.rs` | `BangCommand` gains `exclude_from_context` field, detect `!!` |
| `crates/rho/src/modes/interactive.rs` | I1: persist bang output on `BangDone`, remove `UserMessage` for bangs, restore on resume. I2: use `finish_streaming_with_message`. I3: propagate compaction persist error |
| `crates/rho/src/tui/chat.rs` | Add `finish_streaming_with_message()`, modify `finish_bang()` to return output |
| `crates/rho/src/session/context.rs` | Handle `Message::BashExecution` in context building |

## Tests

- **I1**: Unit test `BashExecutionMessage` serde roundtrip. Unit test context building with/without `exclude_from_context`. Integration: create session, append bang, reopen, verify output restored.
- **I2**: Verify `finish_streaming_with_message` leaves `items` populated (no empty gap).
- **I3**: Verify error path shows message (manual verification; the error path is a simple match).
