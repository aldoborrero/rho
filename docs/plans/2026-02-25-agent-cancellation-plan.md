# Agent Cancellation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire `tokio_util::CancellationToken` end-to-end so Ctrl+C stops the LLM stream, kills in-flight tools, and emits `AgentOutcome::Cancelled`.

**Architecture:** Bottom-up through the dependency graph: first change the `Tool` trait and `ToolRegistry` in `rho-agent`, then the agent loop checkpoints, then adapt `rho-tools` public functions to accept external `CancelToken`, then bridge in each tool implementation in `rho`, and finally wire everything in `interactive.rs`.

**Tech Stack:** `tokio-util` (CancellationToken), `rho-tools::cancel` (CancelToken/AbortToken bridge), `async-trait`

**Design doc:** `docs/plans/2026-02-25-agent-cancellation-design.md`

---

### Task 1: Add `tokio-util` to `rho-agent` and update `Tool` trait + `ToolRegistry`

**Files:**
- Modify: `crates/rho-agent/Cargo.toml`
- Modify: `crates/rho-agent/src/tools.rs`
- Modify: `crates/rho-agent/src/registry.rs`

**Step 1: Add `tokio-util` dependency**

In `crates/rho-agent/Cargo.toml`, add to `[dependencies]`:

```toml
tokio-util = "0.7"
```

**Step 2: Update Tool trait**

In `crates/rho-agent/src/tools.rs`, add import and change `execute` signature:

```rust
use std::path::Path;

use tokio_util::sync::CancellationToken;

// ...

#[async_trait]
pub trait Tool: Send + Sync {
	fn name(&self) -> &'static str;
	fn description(&self) -> &'static str;
	fn input_schema(&self) -> serde_json::Value;
	async fn execute(
		&self,
		input: serde_json::Value,
		cwd: &Path,
		cancel: &CancellationToken,
	) -> anyhow::Result<ToolOutput>;
}
```

**Step 3: Update ToolRegistry::execute**

In `crates/rho-agent/src/registry.rs`, add import and update `execute`:

```rust
use tokio_util::sync::CancellationToken;

// ... in impl ToolRegistry ...

pub async fn execute(
	&self,
	name: &str,
	input: serde_json::Value,
	cwd: &Path,
	cancel: &CancellationToken,
) -> anyhow::Result<ToolOutput> {
	let tool = self
		.tools
		.get(name)
		.ok_or_else(|| anyhow::anyhow!("Unknown tool: {name}"))?;
	tool.execute(input, cwd, cancel).await
}
```

**Step 4: Verify compilation fails (expected — callers not updated yet)**

Run: `cargo check -p rho-agent 2>&1 | head -20`

Expected: Compiles successfully (no callers inside rho-agent call `execute` with wrong arity). The agent_loop.rs call will fail in the next task.

Run: `cargo check -p rho 2>&1 | head -5`

Expected: FAIL — all 11 tool implementations have wrong `execute` signature.

**Step 5: Commit**

```bash
git add crates/rho-agent/Cargo.toml crates/rho-agent/src/tools.rs crates/rho-agent/src/registry.rs
git commit -m "refactor: add CancellationToken param to Tool trait and ToolRegistry"
```

---

### Task 2: Add cancellation checkpoints to `run_agent_loop`

**Files:**
- Modify: `crates/rho-agent/src/agent_loop.rs`

**Step 1: Add `abort` field to `AgentConfig`**

Add `tokio_util` import and new field:

```rust
use tokio_util::sync::CancellationToken;

pub struct AgentConfig {
	pub system_prompt: String,
	pub max_tokens:    u32,
	pub thinking:      ThinkingLevel,
	pub retry:         rho_ai::RetryConfig,
	pub cwd:           PathBuf,
	pub api_key:       Option<String>,
	pub temperature:   Option<f32>,
	pub abort:         Option<CancellationToken>,
}
```

**Step 2: Add cancellation helper**

Add a helper function to check the abort token and emit `Cancelled`:

```rust
/// Check if the abort token is cancelled and emit `Cancelled` if so.
async fn check_cancelled(
	abort: &Option<CancellationToken>,
	event_tx: &mpsc::Sender<AgentEvent>,
) -> Option<AgentOutcome> {
	if abort.as_ref().is_some_and(CancellationToken::is_cancelled) {
		let outcome = AgentOutcome::Cancelled;
		let _ = event_tx.send(AgentEvent::Done(outcome.clone())).await;
		return Some(outcome);
	}
	None
}
```

**Step 3: Wire abort to StreamOptions**

In `run_agent_loop`, where `StreamOptions` is constructed (around line 72-82), change `..Default::default()` to include `abort`:

```rust
let options = rho_ai::types::StreamOptions {
	api_key: config.api_key.clone(),
	max_tokens: Some(max_tokens_for(config.thinking, config.max_tokens)),
	reasoning: thinking_to_reasoning(config.thinking),
	temperature,
	retry: config.retry.clone(),
	abort: config.abort.clone(),
	..Default::default()
};
```

**Step 4: Add checkpoint 1 — before TurnStart (top of outer loop)**

At the top of the outer loop (around line 59), before the `TurnStart` event:

```rust
loop {
	// Checkpoint 1: before starting a new turn.
	if let Some(outcome) = check_cancelled(&config.abort, &event_tx).await {
		return outcome;
	}

	turn += 1;
	let _ = event_tx.send(AgentEvent::TurnStart { turn }).await;
	// ...
```

**Step 5: Add checkpoint 2 — before each tool execution**

Inside the tool call loop (around line 172-178), before each `tools.execute()`:

```rust
for block in &tool_calls {
	if let ContentBlock::ToolUse { id, name, input } = block {
		// Checkpoint 2: before each tool execution.
		if let Some(outcome) = check_cancelled(&config.abort, &event_tx).await {
			return outcome;
		}

		let _ = event_tx
			.send(AgentEvent::ToolCallStart { id: id.clone(), name: name.clone() })
			.await;
```

**Step 6: Pass cancel token to tools.execute()**

Change the `tools.execute()` call to pass the abort token (creating a default if None):

```rust
let cancel = config
	.abort
	.clone()
	.unwrap_or_default();
let tool_result = tools.execute(name, input.clone(), &config.cwd, &cancel).await;
```

**Step 7: Add checkpoint 3 — before continuing to next LLM turn**

After the tool call loop, before the `continue` (around line 211):

```rust
		} // end for block in tool_calls
		// Checkpoint 3: before sending tool results back to LLM.
		if let Some(outcome) = check_cancelled(&config.abort, &event_tx).await {
			return outcome;
		}
		continue;
```

**Step 8: Verify rho-agent compiles and tests pass**

Run: `cargo test -p rho-agent`

Expected: All 15 existing tests pass. The two tests (`test_thinking_to_reasoning`, `test_max_tokens_for_thinking`) don't touch tool execution.

Run: `cargo check -p rho 2>&1 | head -5`

Expected: Still FAIL — tool implementations not updated yet.

**Step 9: Commit**

```bash
git add crates/rho-agent/src/agent_loop.rs
git commit -m "feat: add cancellation checkpoints to agent loop"
```

---

### Task 3: Update `rho-tools` public functions to accept `CancelToken`

**Files:**
- Modify: `crates/rho-tools/src/shell.rs` — `execute_shell()` signature
- Modify: `crates/rho-tools/src/grep.rs` — `grep()` signature
- Modify: `crates/rho-tools/src/glob.rs` — `glob()` signature
- Modify: `crates/rho-tools/src/fd.rs` — `fuzzy_find()` signature

**Step 1: Update `execute_shell`**

In `crates/rho-tools/src/shell.rs`, change `execute_shell` (line 237-248) to accept a `CancelToken` instead of creating one:

```rust
pub async fn execute_shell(
	options: ShellExecuteOptions,
	on_chunk: Option<Box<dyn Fn(String) + Send + Sync>>,
	ct: cancel::CancelToken,
) -> Result<ShellExecuteResult> {
	let config =
		ShellConfig { session_env: options.session_env, snapshot_path: options.snapshot_path };
	let run_config =
		ShellRunConfig { command: options.command, cwd: options.cwd, env: options.env };

	run_shell_oneshot(config, run_config, on_chunk, ct).await
}
```

Remove the `let ct = cancel::CancelToken::new(options.timeout_ms);` line that was there before.

Also remove `timeout_ms` from `ShellExecuteOptions` if it's no longer needed there (the caller now creates the `CancelToken` with the timeout). Check if `timeout_ms` is used elsewhere in the struct — if it's only used to create the `CancelToken`, remove it.

**Step 2: Update `grep`**

In `crates/rho-tools/src/grep.rs`, change `grep()` (line 922) to accept `CancelToken`:

```rust
pub fn grep(
	options: GrepOptions,
	on_match: Option<&dyn Fn(&GrepMatch)>,
	ct: CancelToken,
) -> Result<GrepResult> {
	// ... extract fields from options (unchanged) ...

	let config = GrepConfig { /* ... unchanged ... */ };

	// Remove: let ct = CancelToken::new(timeout_ms);
	grep_sync(config, on_match, ct)
}
```

Remove `timeout_ms` from the options destructuring and the `CancelToken::new()` call.

**Step 3: Update `glob`**

In `crates/rho-tools/src/glob.rs`, change `glob()` (line 212) to accept `CancelToken`:

```rust
pub fn glob(
	options: GlobOptions,
	on_match: Option<&dyn Fn(&GlobMatch)>,
	ct: CancelToken,
) -> Result<GlobResult> {
	// ... extract fields from options (unchanged) ...
	// Remove: let ct = CancelToken::new(timeout_ms);

	run_glob(/* ... unchanged ... */, ct)
}
```

**Step 4: Update `fuzzy_find`**

In `crates/rho-tools/src/fd.rs`, change `fuzzy_find()` (line 233) to accept `CancelToken`:

```rust
pub fn fuzzy_find(options: FuzzyFindOptions, ct: CancelToken) -> Result<FuzzyFindResult> {
	// ... extract fields ...
	// Remove: let ct = CancelToken::new(timeout_ms);
	let config = FuzzyFindConfig { /* ... */ };
	fuzzy_find_sync(config, ct)
}
```

**Step 5: Check for internal callers within rho-tools**

Search for any calls to these functions within `rho-tools` itself (e.g., tests). Update their call sites to pass `CancelToken::new(None)` or `CancelToken::new(Some(timeout))`.

Run: `cargo check -p rho-tools 2>&1 | head -30`

Fix any compilation errors from internal callers or tests.

**Step 6: Run rho-tools tests**

Run: `cargo test -p rho-tools`

Expected: All tests pass (with updated call sites).

**Step 7: Commit**

```bash
git add crates/rho-tools/src/shell.rs crates/rho-tools/src/grep.rs crates/rho-tools/src/glob.rs crates/rho-tools/src/fd.rs
# Include any other files changed (test files, options structs)
git commit -m "refactor: accept external CancelToken in shell/grep/glob/fd"
```

---

### Task 4: Update fast tool implementations (7 tools)

**Files:**
- Modify: `crates/rho/src/tools/read.rs`
- Modify: `crates/rho/src/tools/write.rs`
- Modify: `crates/rho/src/tools/clipboard.rs`
- Modify: `crates/rho/src/tools/html_to_markdown.rs`
- Modify: `crates/rho/src/tools/image.rs`
- Modify: `crates/rho/src/tools/process.rs`
- Modify: `crates/rho/src/tools/workmux.rs`

**Step 1: Add `_cancel` parameter to all 7 fast tools**

Each tool's `execute` method gets a new parameter. The pattern is identical for all:

```rust
use tokio_util::sync::CancellationToken;

async fn execute(
	&self,
	input: Value,
	cwd: &Path,           // or _cwd for tools that don't use it
	_cancel: &CancellationToken,
) -> anyhow::Result<ToolOutput> {
	// ... body unchanged ...
}
```

For tools that already ignore `cwd` (clipboard, html_to_markdown, process, workmux), the existing `_cwd` pattern shows the convention.

**Step 2: Verify they compile**

Run: `cargo check -p rho 2>&1 | grep "error" | head -10`

Expected: Only errors from the 4 long-running tools (bash, grep, find, fuzzy_find) which still have the old signature.

**Step 3: Commit**

```bash
git add crates/rho/src/tools/read.rs crates/rho/src/tools/write.rs crates/rho/src/tools/clipboard.rs crates/rho/src/tools/html_to_markdown.rs crates/rho/src/tools/image.rs crates/rho/src/tools/process.rs crates/rho/src/tools/workmux.rs
git commit -m "refactor: add CancellationToken param to fast tool implementations"
```

---

### Task 5: Update long-running tool implementations with bridge pattern

**Files:**
- Modify: `crates/rho/src/tools/bash.rs`
- Modify: `crates/rho/src/tools/grep.rs`
- Modify: `crates/rho/src/tools/find.rs`
- Modify: `crates/rho/src/tools/fuzzy_find.rs`

**Step 1: Update bash tool with bridge**

In `crates/rho/src/tools/bash.rs`, update the `execute` method:

```rust
use rho_tools::cancel;
use tokio_util::sync::CancellationToken;

async fn execute(
	&self,
	input: Value,
	cwd: &Path,
	cancel_token: &CancellationToken,
) -> anyhow::Result<ToolOutput> {
	// ... parse input (unchanged up to execute_shell call) ...

	// Create internal CancelToken with the tool's timeout.
	let ct = cancel::CancelToken::new(Some(u32::try_from(timeout_secs * 1000).unwrap_or(u32::MAX)));
	let internal_abort = ct.emplace_abort_token();

	// Bridge: external CancellationToken → internal CancelToken.
	let external = cancel_token.clone();
	let bridge = tokio::spawn(async move {
		external.cancelled().await;
		internal_abort.abort(cancel::AbortReason::Signal);
	});

	let options = ShellExecuteOptions {
		command:       command.to_owned(),
		cwd:           Some(cwd.to_string_lossy().into_owned()),
		env:           None,
		session_env:   None,
		// timeout_ms removed — handled by CancelToken
		snapshot_path: None,
	};

	let result = execute_shell(options, Some(on_chunk), ct).await;

	bridge.abort(); // Clean up bridge if tool finished normally.

	// ... handle result (unchanged) ...
}
```

Note: If `timeout_ms` was removed from `ShellExecuteOptions` in Task 3, adjust accordingly. If it was kept for backward compat, just don't set it (or set to `None`).

**Step 2: Update grep tool with bridge**

In `crates/rho/src/tools/grep.rs`, update the `execute` method:

```rust
use rho_tools::cancel;
use tokio_util::sync::CancellationToken;

async fn execute(
	&self,
	input: Value,
	cwd: &Path,
	cancel_token: &CancellationToken,
) -> anyhow::Result<ToolOutput> {
	// ... parse input (unchanged) ...

	// Create internal CancelToken with timeout.
	let ct = cancel::CancelToken::new(timeout_ms);
	let internal_abort = ct.emplace_abort_token();

	// Bridge: external CancellationToken → internal CancelToken.
	let external = cancel_token.clone();
	let bridge = tokio::spawn(async move {
		external.cancelled().await;
		internal_abort.abort(cancel::AbortReason::Signal);
	});

	let result = tokio::task::spawn_blocking(move || {
		rho_tools::grep::grep(options, None, ct)
	})
	.await
	.map_err(|e| anyhow::anyhow!("Grep task panicked: {e}"))?;

	bridge.abort();

	// ... format result (unchanged) ...
}
```

**Step 3: Update find tool with bridge**

In `crates/rho/src/tools/find.rs`, same pattern:

```rust
use rho_tools::cancel;
use tokio_util::sync::CancellationToken;

async fn execute(
	&self,
	input: Value,
	cwd: &Path,
	cancel_token: &CancellationToken,
) -> anyhow::Result<ToolOutput> {
	// ... parse input (unchanged) ...

	let ct = cancel::CancelToken::new(timeout_ms);
	let internal_abort = ct.emplace_abort_token();

	let external = cancel_token.clone();
	let bridge = tokio::spawn(async move {
		external.cancelled().await;
		internal_abort.abort(cancel::AbortReason::Signal);
	});

	let result = tokio::task::spawn_blocking(move || {
		rho_tools::glob::glob(options, None, ct)
	})
	.await
	.map_err(|e| anyhow::anyhow!("Find task panicked: {e}"))?;

	bridge.abort();

	// ... format result (unchanged) ...
}
```

**Step 4: Update fuzzy_find tool with bridge**

In `crates/rho/src/tools/fuzzy_find.rs`, same pattern:

```rust
use rho_tools::cancel;
use tokio_util::sync::CancellationToken;

async fn execute(
	&self,
	input: Value,
	cwd: &Path,
	cancel_token: &CancellationToken,
) -> anyhow::Result<ToolOutput> {
	// ... parse input (unchanged) ...

	let ct = cancel::CancelToken::new(timeout_ms);
	let internal_abort = ct.emplace_abort_token();

	let external = cancel_token.clone();
	let bridge = tokio::spawn(async move {
		external.cancelled().await;
		internal_abort.abort(cancel::AbortReason::Signal);
	});

	let result = tokio::task::spawn_blocking(move || {
		rho_tools::fd::fuzzy_find(options, ct)
	})
	.await
	.map_err(|e| anyhow::anyhow!("Fuzzy find task panicked: {e}"))?;

	bridge.abort();

	// ... format result (unchanged) ...
}
```

**Step 5: Full compilation check**

Run: `cargo check -p rho 2>&1 | tail -5`

Expected: Compiles successfully — all 11 tool implementations now match the trait.

**Step 6: Commit**

```bash
git add crates/rho/src/tools/bash.rs crates/rho/src/tools/grep.rs crates/rho/src/tools/find.rs crates/rho/src/tools/fuzzy_find.rs
git commit -m "feat: wire cancellation bridge in long-running tools"
```

---

### Task 6: Wire cancellation in `interactive.rs`

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs`

**Step 1: Add `agent_cancel` state and update `cancel_streaming`**

Add the `agent_cancel` variable alongside the existing `agent_generation`:

```rust
let mut agent_generation: u64 = 0;
let mut agent_cancel: Option<tokio_util::sync::CancellationToken> = None;
```

Update `cancel_streaming` to accept and fire the token:

```rust
fn cancel_streaming(
	mode: &mut AppMode,
	app: &mut tui::App,
	terminal: &impl Terminal,
	agent_generation: &mut u64,
	agent_cancel: &mut Option<tokio_util::sync::CancellationToken>,
) {
	*mode = AppMode::Idle;
	*agent_generation += 1;
	if let Some(token) = agent_cancel.take() {
		token.cancel();
	}
	app.chat.finish_streaming();
	app.status.finish_working();
	app.update_status_border(terminal.columns());
}
```

**Step 2: Update `spawn_agent` to create and return a `CancellationToken`**

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
) -> tokio_util::sync::CancellationToken {
	*agent_generation += 1;
	let generation = *agent_generation;
	let cancel = tokio_util::sync::CancellationToken::new();

	// ... forwarding task (unchanged) ...

	let agent_config = AgentConfig {
		system_prompt: system_prompt.to_owned(),
		max_tokens:    settings.agent.max_tokens,
		thinking:      parse_thinking(&settings.agent.thinking),
		retry:         rho_ai::RetryConfig {
			enabled:       true,
			max_retries:   settings.retry.max_retries,
			base_delay_ms: settings.retry.base_delay_ms,
			max_delay_ms:  settings.retry.base_delay_ms * 16,
		},
		cwd:           std::env::current_dir().unwrap_or_default(),
		api_key:       Some(api_key.to_owned()),
		temperature:   Some(settings.agent.temperature),
		abort:         Some(cancel.clone()),
	};

	// ... spawn agent loop task (unchanged) ...

	cancel
}
```

**Step 3: Update all call sites**

Every `spawn_agent(...)` call now stores the returned token:

```rust
// Initial message (around line 388):
agent_cancel = Some(spawn_agent(..., &mut agent_generation));

// User message submission (around line 560):
agent_cancel = Some(spawn_agent(..., &mut agent_generation));
```

Every `cancel_streaming(...)` call passes `&mut agent_cancel`:

```rust
// Ctrl+C handler:
cancel_streaming(&mut mode, &mut app, &terminal, &mut agent_generation, &mut agent_cancel);

// Escape handler:
cancel_streaming(&mut mode, &mut app, &terminal, &mut agent_generation, &mut agent_cancel);
```

**Step 4: Handle `AgentOutcome::Cancelled` in the Done handler**

In the `AgentEvent::Done` match arm, clear `agent_cancel` and handle the new variant:

```rust
AgentEvent::Done(outcome) => {
	mode = AppMode::Idle;
	agent_cancel = None;
	app.chat.finish_streaming();
	app.status.finish_working();
	app.update_status_border(terminal.columns());

	// Auto-compaction only for non-cancelled outcomes.
	let maybe_usage = match &outcome {
		AgentOutcome::Stop { usage } | AgentOutcome::MaxTokens { usage } => {
			usage.as_ref()
		},
		_ => None,
	};
	// ... auto-compaction code (unchanged) ...

	match outcome {
		AgentOutcome::Cancelled => {},  // Clean exit, nothing to show.
		AgentOutcome::MaxTokens { .. } => {
			show_chat_message(
				&mut app,
				"Warning: response truncated (max tokens reached).",
			);
		},
		AgentOutcome::Failed { error } => {
			show_chat_message(&mut app, &format!("Error: {error}"));
		},
		_ => {},
	}
},
```

**Step 5: Verify compilation**

Run: `cargo check -p rho`

Expected: Compiles successfully.

**Step 6: Commit**

```bash
git add crates/rho/src/modes/interactive.rs
git commit -m "feat: wire CancellationToken through spawn_agent and cancel_streaming"
```

---

### Task 7: Full verification

**Step 1: Run all tests**

```bash
cargo test -p rho-agent
cargo test -p rho-tools
cargo test -p rho
```

Expected: All tests pass.

**Step 2: Run clippy**

```bash
cargo clippy --workspace 2>&1 | grep "error"
```

Expected: No new errors. Pre-existing warnings are OK.

**Step 3: Check for unused imports**

```bash
cargo clippy --workspace 2>&1 | grep -i "unused import" | grep -E "(interactive|bash|grep|find|fuzzy_find|agent_loop|tools\.rs|registry)"
```

Expected: No unused imports in modified files.

**Step 4: Commit any fixups**

If clippy or tests revealed issues, fix and commit:

```bash
git add -u
git commit -m "fix: address clippy warnings from cancellation changes"
```
