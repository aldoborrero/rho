# I1-I2-I3 Bugfixes Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix three bugs: bang output not persisted (I1), streaming flash on MessageComplete (I2), auto-compaction error silently swallowed (I3).

**Architecture:** I2 and I3 are small, isolated changes in `interactive.rs` and `tui/chat.rs`. I1 is larger: adds a new `Message::BashExecution` variant in `rho-agent`, updates input routing for `!!`, changes the bang event flow to capture output, persists on completion, handles resume, and adds context building support.

**Tech Stack:** Rust, serde (JSON serialization), tokio (async runtime)

**Design doc:** `docs/plans/2026-02-26-i1-i2-i3-bugfixes-design.md`

---

### Task 1: I3 — Propagate auto-compaction persistence error

The simplest fix. Do it first to warm up.

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs:740-752`

**Step 1: Replace `let _ =` with match**

In `crates/rho/src/modes/interactive.rs`, find the `AgentEvent::Done` handler's auto-compaction block (around line 740). Replace:

```rust
									let _ = session.append_compaction(
										&result.summary,
										result.short_summary.as_deref(),
										&result.first_kept_entry_id,
										result.tokens_before,
										result.details,
									);
									let msg = result
										.short_summary
										.as_deref()
										.unwrap_or("Conversation compacted.");
									show_chat_message(&mut app, &format!("Auto-compacted: {msg}"));
```

With:

```rust
									match session.append_compaction(
										&result.summary,
										result.short_summary.as_deref(),
										&result.first_kept_entry_id,
										result.tokens_before,
										result.details,
									) {
										Ok(()) => {
											let msg = result
												.short_summary
												.as_deref()
												.unwrap_or("Conversation compacted.");
											show_chat_message(&mut app, &format!("Auto-compacted: {msg}"));
										},
										Err(e) => {
											show_chat_message(
												&mut app,
												&format!("Auto-compaction succeeded but failed to persist: {e}"),
											);
										},
									}
```

**Step 2: Verify it compiles**

Run: `cargo build -p rho 2>&1 | tail -5`
Expected: compiles cleanly

**Step 3: Commit**

```bash
git add crates/rho/src/modes/interactive.rs
git commit -m "fix(I3): propagate auto-compaction persistence error to user"
```

---

### Task 2: I2 — Fix MessageComplete streaming flash

**Files:**
- Modify: `crates/rho/src/tui/chat.rs:158` (add new method after `finish_streaming`)
- Modify: `crates/rho/src/modes/interactive.rs:688-689` (use new method)

**Step 1: Add `finish_streaming_with_message()` to `ChatComponent`**

In `crates/rho/src/tui/chat.rs`, add this method right after `finish_streaming()` (after line 164):

```rust
	/// Atomically finish streaming and commit the final message.
	///
	/// Unlike calling `finish_streaming()` then `add_message()` separately,
	/// this ensures the streaming buffer and committed message are never
	/// both absent in the same render frame (no visual flash).
	pub fn finish_streaming_with_message(&mut self, message: Message) {
		self.is_streaming = false;
		self.streaming_text.clear();
		self.streaming_thinking.clear();
		self.loader.stop();
		self.tool_executing = None;
		self.items.push(ChatItem::Message(message));
	}
```

**Step 2: Update MessageComplete handler in `interactive.rs`**

In `crates/rho/src/modes/interactive.rs`, in the `AgentEvent::MessageComplete` arm (around line 688), replace:

```rust
					app.chat.finish_streaming();
					app.chat.add_message(Message::Assistant(message));
```

With:

```rust
					app.chat.finish_streaming_with_message(Message::Assistant(message));
```

**Step 3: Verify it compiles**

Run: `cargo build -p rho 2>&1 | tail -5`
Expected: compiles cleanly

**Step 4: Commit**

```bash
git add crates/rho/src/tui/chat.rs crates/rho/src/modes/interactive.rs
git commit -m "fix(I2): eliminate streaming flash on MessageComplete with atomic transition"
```

---

### Task 3: I1a — Add `BashExecutionMessage` type and `Message` variant

**Files:**
- Modify: `crates/rho-agent/src/types.rs:1-14` (add struct and enum variant)
- Test: same file, test module

**Step 1: Write the failing test**

In `crates/rho-agent/src/types.rs`, add to the existing `mod tests` block:

```rust
	#[test]
	fn test_bash_execution_message_roundtrip() {
		let msg = Message::BashExecution(BashExecutionMessage {
			command:              "ls -la".to_owned(),
			output:               "total 0\ndrwxr-xr-x 2 user user 40 Jan 1 00:00 .\n".to_owned(),
			exit_code:            Some(0),
			cancelled:            false,
			truncated:            false,
			exclude_from_context: false,
			timestamp:            1706_000_000,
		});
		let json = serde_json::to_string(&msg).unwrap();
		assert!(json.contains("\"role\":\"bashExecution\""), "tag should be bashExecution");
		let parsed: Message = serde_json::from_str(&json).unwrap();
		match parsed {
			Message::BashExecution(b) => {
				assert_eq!(b.command, "ls -la");
				assert_eq!(b.exit_code, Some(0));
				assert!(!b.exclude_from_context);
			},
			_ => panic!("Expected BashExecution message"),
		}
	}

	#[test]
	fn test_bash_execution_exclude_from_context() {
		let msg = Message::BashExecution(BashExecutionMessage {
			command:              "pwd".to_owned(),
			output:               "/home/user".to_owned(),
			exit_code:            Some(0),
			cancelled:            false,
			truncated:            false,
			exclude_from_context: true,
			timestamp:            1706_000_000,
		});
		let json = serde_json::to_string(&msg).unwrap();
		let parsed: Message = serde_json::from_str(&json).unwrap();
		match parsed {
			Message::BashExecution(b) => assert!(b.exclude_from_context),
			_ => panic!("Expected BashExecution message"),
		}
	}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rho-agent -- test_bash_execution 2>&1 | tail -10`
Expected: FAIL — `BashExecutionMessage` and `Message::BashExecution` not defined

**Step 3: Add the struct and variant**

In `crates/rho-agent/src/types.rs`, add the struct after `ToolResultMessage` (after line 36):

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

Add the variant to the `Message` enum (after the `ToolResult` variant):

```rust
	#[serde(rename = "bashExecution")]
	BashExecution(BashExecutionMessage),
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p rho-agent -- test_bash_execution 2>&1 | tail -10`
Expected: PASS

**Step 5: Fix downstream compilation**

Adding a variant to `Message` will break exhaustive matches throughout the codebase. Run `cargo build --workspace 2>&1 | head -40` to find them. The key files that need `Message::BashExecution` match arms:

- `crates/rho-agent/src/agent_loop.rs` — message conversion to provider format (skip or panic for now — bang messages don't go to the LLM directly)
- `crates/rho-ai/src/types.rs` — if `Message` is defined here too or re-exported (it's re-exported via `crates/rho/src/ai/types.rs` which does `pub use rho_agent::types::*`)
- `crates/rho/src/session/context.rs` — handle in context building (next task)
- `crates/rho/src/tui/chat.rs` — rendering match arms
- `crates/rho/src/compaction/serialize.rs` — serialization for compaction

For each broken match, add a `Message::BashExecution(_) => { /* handled in next tasks */ }` arm or the appropriate logic. Use `cargo build --workspace` iteratively until it compiles.

**Step 6: Run all tests**

Run: `cargo test -p rho-agent 2>&1 | tail -5`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/rho-agent/src/types.rs
# Also add any files touched for exhaustive match fixes
git commit -m "feat(I1): add BashExecutionMessage type and Message::BashExecution variant"
```

---

### Task 4: I1b — Update input routing for `!!`

**Files:**
- Modify: `crates/rho/src/modes/input.rs:8-42` (change `BangCommand` variant, update `route_input`)
- Test: same file, test module

**Step 1: Write the failing tests**

In `crates/rho/src/modes/input.rs`, add to the test module:

```rust
	#[test]
	fn double_bang_is_bang_excluded() {
		match route_input("!!ls -la") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "ls -la");
				assert!(exclude_from_context);
			},
			_ => panic!("Expected BangCommand with exclude_from_context"),
		}
	}

	#[test]
	fn single_bang_not_excluded() {
		match route_input("!pwd") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "pwd");
				assert!(!exclude_from_context);
			},
			_ => panic!("Expected BangCommand"),
		}
	}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rho -- modes::input::tests::double_bang 2>&1 | tail -10`
Expected: FAIL — pattern doesn't match (still the old `BangCommand(&str)` variant)

**Step 3: Update the enum and `route_input()`**

In `crates/rho/src/modes/input.rs`, change the `BangCommand` variant from:

```rust
	/// A `!`-prefixed shell command.
	BangCommand(&'a str),
```

To:

```rust
	/// A `!`-prefixed shell command (`!!` excludes from LLM context).
	BangCommand { cmd: &'a str, exclude_from_context: bool },
```

Change the routing logic from:

```rust
	if text.starts_with('!') && !text.starts_with("!!") {
		return InputAction::BangCommand(&text[1..]);
	}
```

To:

```rust
	if text.starts_with("!!") {
		return InputAction::BangCommand { cmd: &text[2..], exclude_from_context: true };
	}
	if text.starts_with('!') {
		return InputAction::BangCommand { cmd: &text[1..], exclude_from_context: false };
	}
```

Note: `!!` check comes first since `!!` also starts with `!`.

**Step 4: Fix existing tests**

Update the existing `bang_command` test to use the new pattern:

```rust
	#[test]
	fn bang_command() {
		match route_input("!ls -la") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "ls -la");
				assert!(!exclude_from_context);
			},
			_ => panic!("Expected BangCommand"),
		}
	}
```

Update `double_bang_is_user_message` — this test was for the old behavior where `!!` was treated as a user message. It should now test the new behavior (or be removed and replaced by the test in Step 1). Remove the old test.

Update `bang_single_char` similarly:

```rust
	#[test]
	fn bang_single_char() {
		match route_input("!x") {
			InputAction::BangCommand { cmd, exclude_from_context } => {
				assert_eq!(cmd, "x");
				assert!(!exclude_from_context);
			},
			_ => panic!("Expected BangCommand"),
		}
	}
```

**Step 5: Fix `interactive.rs` match arms**

In `crates/rho/src/modes/interactive.rs`, update the `InputAction::BangCommand(cmd)` match arm to use the new destructuring pattern:

```rust
InputAction::BangCommand { cmd, exclude_from_context } => {
```

For now, just destructure — `exclude_from_context` will be used in a later task.

Also update the chat display of the user command to show `!!` prefix for excluded commands:

```rust
	app.chat.add_message(Message::User(UserMessage {
		content: format!("{}{}",
			if exclude_from_context { "!!" } else { "!" },
			cmd
		),
	}));
```

**Step 6: Run tests**

Run: `cargo test -p rho -- modes::input 2>&1 | tail -10`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/rho/src/modes/input.rs crates/rho/src/modes/interactive.rs
git commit -m "feat(I1): support !! for exclude-from-context bang commands"
```

---

### Task 5: I1c — Capture bang output and persist on `BangDone`

This is the core persistence change.

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs:32-43` (AppEvent::BangDone fields)
- Modify: `crates/rho/src/modes/interactive.rs:552-593` (bang spawn)
- Modify: `crates/rho/src/modes/interactive.rs:776-787` (BangDone handler)
- Modify: `crates/rho/src/tui/chat.rs:133-140` (finish_bang return value)
- Modify: `crates/rho/src/commands/dispatch.rs:55-80` (return exit_code)

**Step 1: Change `AppEvent::BangDone` to carry result data**

In `crates/rho/src/modes/interactive.rs`, change:

```rust
	/// Bang command completed.
	BangDone { is_error: bool },
```

To:

```rust
	/// Bang command completed.
	BangDone {
		exit_code: Option<i32>,
		cancelled: bool,
	},
```

**Step 2: Update `execute_bang_streaming` to return exit_code**

In `crates/rho/src/commands/dispatch.rs`, change the return after the `tokio::select!`:

Currently the spawned task in `interactive.rs` (line 579-591) does:
```rust
let result = tokio::select! { ... };
let is_error = result.map_or(true, |o| o.is_error);
let _ = done_tx.send(AppEvent::BangDone { is_error }).await;
```

But `ToolOutput` doesn't carry exit_code. We need `execute_bang_streaming` to return the `ShellExecuteResult` directly. Change its return type from `Result<ToolOutput>` to `Result<(ToolOutput, Option<i32>, bool)>` where the extra values are `exit_code` and `cancelled`:

In `crates/rho/src/commands/dispatch.rs`, change:

```rust
pub async fn execute_bang_streaming<F>(
	command: &str,
	_tools: &ToolRegistry,
	on_chunk: F,
) -> anyhow::Result<rho_agent::tools::ToolOutput>
```

To:

```rust
/// Result of a streaming bang command execution.
pub struct BangResult {
	pub is_error:  bool,
	pub exit_code: Option<i32>,
	pub cancelled: bool,
}

pub async fn execute_bang_streaming<F>(
	command: &str,
	_tools: &ToolRegistry,
	on_chunk: F,
) -> anyhow::Result<BangResult>
```

And change the return:

```rust
	let is_error = result.exit_code.is_none_or(|c| c != 0);
	Ok(BangResult {
		is_error,
		exit_code: result.exit_code,
		cancelled: result.cancelled,
	})
```

Update `mod.rs` re-export if needed: `pub use dispatch::{BangResult, execute_bang, execute_bang_streaming, execute_command};`

**Step 3: Update the spawned task in `interactive.rs`**

In the `BangCommand` handler (around line 569), update the spawned task to send the new `BangDone`:

```rust
tokio::spawn(async move {
	let shell_fut = crate::commands::execute_bang_streaming(
		&cmd_owned,
		&bang_tools,
		move |chunk| {
			let _ = chunk_tx.try_send(AppEvent::BangChunk(chunk));
		},
	);
	let result = tokio::select! {
		r = shell_fut => r,
		_ = cancel_rx => {
			Ok(crate::commands::BangResult {
				is_error: true,
				exit_code: None,
				cancelled: true,
			})
		}
	};
	let (exit_code, cancelled) = match result {
		Ok(r) => (r.exit_code, r.cancelled),
		Err(_) => (None, false),
	};
	let _ = done_tx.send(AppEvent::BangDone { exit_code, cancelled }).await;
});
```

**Step 4: Make `finish_bang()` return the bang data**

In `crates/rho/src/tui/chat.rs`, change `finish_bang`:

```rust
	/// Finish the streaming bang command and commit it to the display.
	///
	/// Returns the command and accumulated output so the caller can persist them.
	pub fn finish_bang(&mut self, is_error: bool) -> Option<(String, String)> {
		if let Some(mut bang) = self.streaming_bang.take() {
			bang.is_error = is_error;
			let command = bang.command.clone();
			let output = bang.output.clone();
			self.items.push(ChatItem::Bang(bang));
			self.loader.stop();
			Some((command, output))
		} else {
			None
		}
	}
```

**Step 5: Update `BangDone` handler to persist**

In `crates/rho/src/modes/interactive.rs`, change the `BangDone` handler:

```rust
			Some(AppEvent::BangDone { exit_code, cancelled }) => {
				if matches!(mode, AppMode::BangRunning) {
					let is_error = exit_code.is_none_or(|c| c != 0);
					if let Some((command, output)) = app.chat.finish_bang(is_error) {
						let bash_msg = Message::BashExecution(BashExecutionMessage {
							command,
							output,
							exit_code,
							cancelled,
							truncated:            false,
							exclude_from_context: bang_exclude_from_context,
							timestamp:            chrono::Utc::now().timestamp(),
						});
						session.append(bash_msg).await?;
					}
					mode = AppMode::Idle;
				}
			},
```

Note: `bang_exclude_from_context` needs to be captured when the bang starts. Add a local variable in the event loop state (near `bang_cancel`):

```rust
	let mut bang_exclude_from_context: bool = false;
```

Set it in the `BangCommand` handler:

```rust
	bang_exclude_from_context = exclude_from_context;
```

**Step 6: Remove the `UserMessage` persistence for bang commands**

In the `BangCommand` handler, the current code does:
```rust
app.chat.add_message(Message::User(UserMessage {
    content: format!("!{cmd}"),
}));
```

Keep this for display purposes (shows the user's input in chat), but do NOT call `session.append()` for it — the `BashExecutionMessage` will be the single source of truth for persistence. Currently there's no `session.append()` call for the user message in the bang path, so this is already correct. The `add_message` is display-only.

**Step 7: Verify it compiles and tests pass**

Run: `cargo build -p rho 2>&1 | tail -5`
Run: `cargo test -p rho 2>&1 | tail -10`
Expected: compiles and passes

**Step 8: Commit**

```bash
git add crates/rho/src/modes/interactive.rs crates/rho/src/tui/chat.rs crates/rho/src/commands/dispatch.rs crates/rho/src/commands/mod.rs
git commit -m "feat(I1): persist bang command output to session on BangDone"
```

---

### Task 6: I1d — Handle `BashExecution` in context building

**Files:**
- Modify: `crates/rho/src/session/context.rs:65-96`
- Test: same file, test module

**Step 1: Write the failing tests**

In `crates/rho/src/session/context.rs`, add to the test module:

```rust
	#[test]
	fn test_build_context_bash_execution_included() {
		use crate::ai::types::BashExecutionMessage;

		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::Message(SessionMessageEntry {
			id:        "a2".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			message:   Message::BashExecution(BashExecutionMessage {
				command:              "ls".to_owned(),
				output:               "file.txt".to_owned(),
				exit_code:            Some(0),
				cancelled:            false,
				truncated:            false,
				exclude_from_context: false,
				timestamp:            1706_000_000,
			}),
		});
		let e3 = msg_entry("a3", Some("a2"), "After bash");

		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		// Should have 3 messages: Hello, bash output (as user msg), After bash
		assert_eq!(ctx.messages.len(), 3);
		match &ctx.messages[1] {
			Message::User(u) => {
				assert!(u.content.contains("ls"), "Should contain command");
				assert!(u.content.contains("file.txt"), "Should contain output");
			},
			_ => panic!("BashExecution should be converted to User message"),
		}
	}

	#[test]
	fn test_build_context_bash_execution_excluded() {
		use crate::ai::types::BashExecutionMessage;

		let e1 = msg_entry("a1", None, "Hello");
		let e2 = SessionEntry::Message(SessionMessageEntry {
			id:        "a2".to_owned(),
			parent_id: Some("a1".to_owned()),
			timestamp: ts(),
			message:   Message::BashExecution(BashExecutionMessage {
				command:              "ls".to_owned(),
				output:               "file.txt".to_owned(),
				exit_code:            Some(0),
				cancelled:            false,
				truncated:            false,
				exclude_from_context: true,
				timestamp:            1706_000_000,
			}),
		});
		let e3 = msg_entry("a3", Some("a2"), "After bash");

		let branch: Vec<&SessionEntry> = vec![&e3, &e2, &e1];
		let ctx = build_context(&branch);

		// Should have 2 messages: Hello, After bash (bash excluded)
		assert_eq!(ctx.messages.len(), 2);
	}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rho -- session::context::tests::test_build_context_bash_execution 2>&1 | tail -10`
Expected: FAIL (or compile error if the match isn't exhaustive yet)

**Step 3: Update `build_context` to handle `BashExecution`**

In `crates/rho/src/session/context.rs`, in the `SessionEntry::Message` match arm, the current code does:

```rust
SessionEntry::Message(msg_entry) => {
    messages.push(msg_entry.message.clone());
},
```

Change to:

```rust
SessionEntry::Message(msg_entry) => {
    match &msg_entry.message {
        Message::BashExecution(bash) if bash.exclude_from_context => {
            // Excluded from LLM context (!! command)
        },
        Message::BashExecution(bash) => {
            // Convert to user message for LLM context
            let text = format!(
                "$ {}\n{}{}",
                bash.command,
                bash.output,
                if bash.exit_code.is_none_or(|c| c != 0) {
                    format!(
                        "\n[exit code: {}]",
                        bash.exit_code.map_or("unknown".to_owned(), |c| c.to_string())
                    )
                } else {
                    String::new()
                }
            );
            messages.push(Message::User(UserMessage { content: text }));
        },
        _ => {
            messages.push(msg_entry.message.clone());
        },
    }
},
```

**Step 4: Run tests**

Run: `cargo test -p rho -- session::context::tests 2>&1 | tail -10`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/rho/src/session/context.rs
git commit -m "feat(I1): convert BashExecution to user message in context builder"
```

---

### Task 7: I1e — Restore bang output on session resume

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs:405-408` (resume render loop)

**Step 1: Update the resume loop**

In `crates/rho/src/modes/interactive.rs`, the session resume render loop (around line 405) currently does:

```rust
	for msg in session.messages() {
		app.chat.add_message(msg.clone());
	}
```

Change to:

```rust
	for msg in session.messages() {
		match msg {
			Message::BashExecution(bash) => {
				let is_error = bash.exit_code.is_none_or(|c| c != 0);
				app.chat.add_bang_output(&bash.command, &bash.output, is_error);
			},
			_ => {
				app.chat.add_message(msg.clone());
			},
		}
	}
```

This renders bang results as proper bang output blocks (with the command header and colored output) instead of raw message text.

**Step 2: Verify it compiles**

Run: `cargo build -p rho 2>&1 | tail -5`
Expected: compiles cleanly

**Step 3: Commit**

```bash
git add crates/rho/src/modes/interactive.rs
git commit -m "feat(I1): restore bang output on session resume"
```

---

### Task 8: Final verification

**Step 1: Run full test suite**

Run: `cargo test -p rho 2>&1 | tail -20`
Run: `cargo test -p rho-agent 2>&1 | tail -10`
Expected: all pass (minus the 2 pre-existing platform test failures)

**Step 2: Run clippy**

Run: `cargo clippy --workspace 2>&1 | grep "^error" | head -5`
Expected: no errors

**Step 3: Run fmt**

Run: `cargo fmt --all -- --check 2>&1 | head -5`
Expected: no formatting issues

**Step 4: Quick smoke test summary**

Review the changes holistically:
- I3: `let _ =` replaced with match + error message
- I2: `finish_streaming_with_message()` replaces clear+add
- I1: Full `BashExecution` pipeline: type definition, `!!` support, output capture, persistence, context conversion, resume rendering
