# End-to-End Agent Cancellation

## Problem

`spawn_agent` is fire-and-forget. When the user cancels (Ctrl+C), the agent loop
keeps running — consuming API tokens, executing tools, and writing files. The
generation counter (committed in b677520) prevents stale events from corrupting
the session, but doesn't stop the underlying work.

## Goals

1. Stop the LLM stream immediately on cancel (save API spend)
2. Kill in-flight tool execution — bash processes get SIGTERM, grep/find abort
3. Prevent new tools from starting after cancel fires
4. Emit `AgentOutcome::Cancelled` for clean lifecycle tracking
5. Keep the generation counter as defense-in-depth for event races

## Design

### Cancellation Token Flow

```
User presses Ctrl+C
  |
  +-> cancel_streaming()
  |     +-> agent_generation += 1  (stale event guard)
  |     +-> agent_cancel.take().cancel()  (fire CancellationToken)
  |
  +-> If agent is mid-LLM-stream:
  |     +-> StreamOptions.abort fires
  |           +-> Provider drops HTTP connection
  |                 +-> stream returns Done/Error
  |                       +-> agent loop hits checkpoint -> Cancelled
  |
  +-> If agent is mid-tool-execution:
  |     +-> Bash: bridge task fires AbortToken::abort(Signal)
  |     |     +-> CancelToken::wait() in run_shell_oneshot wakes
  |     |           +-> tokio_cancel.cancel()
  |     |                 +-> terminate_background_jobs() -> SIGTERM
  |     |                       +-> 2s grace -> task.abort() -> SIGKILL
  |     |
  |     +-> Grep/Find/FuzzyFind: bridge task fires AbortToken::abort(Signal)
  |     |     +-> heartbeat() returns Err on next call
  |     |           +-> function returns early
  |     |                 +-> spawn_blocking completes
  |     |
  |     +-> Fast tools (read/write): run to completion (milliseconds)
  |
  +-> Agent loop hits next checkpoint
  |     +-> return AgentOutcome::Cancelled
  |           +-> event_tx sends Done(Cancelled)
  |                 +-> drops agent_tx -> forwarding task exits
  |
  +-> Event loop receives Done(Cancelled)
        +-> filtered by generation guard (if stale) or processed normally
```

### Public Interface Changes

**`rho-agent` — `AgentConfig`:**

```rust
pub struct AgentConfig {
    // ... existing fields ...
    pub abort: Option<tokio_util::sync::CancellationToken>,
}
```

**`rho-agent` — `Tool` trait:**

```rust
async fn execute(
    &self,
    input: Value,
    cwd: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ToolOutput>;
```

**`rho-agent` — `ToolRegistry::execute`:**

```rust
pub async fn execute(
    &self,
    name: &str,
    input: Value,
    cwd: &Path,
    cancel: &CancellationToken,
) -> Result<ToolOutput>;
```

**`rho-tools` — long-running functions take `CancelToken` instead of creating one:**

```rust
// shell.rs
pub async fn execute_shell(
    options: ShellExecuteOptions,
    on_chunk: Option<Box<dyn Fn(String) + Send + Sync>>,
    ct: cancel::CancelToken,
) -> Result<ShellExecuteResult>;

// grep.rs
pub fn grep(options: GrepOptions, on_match: ..., ct: CancelToken) -> Result<GrepResult>;

// glob.rs
pub fn glob(options: GlobOptions, on_match: ..., ct: CancelToken) -> Result<GlobResult>;

// fd.rs
pub fn fuzzy_find(options: FuzzyFindOptions, ct: CancelToken) -> Result<FuzzyFindResult>;
```

### Agent Loop Checkpoints

Three cancellation checkpoints in `run_agent_loop`:

1. **Before TurnStart** — prevents starting a new LLM turn after cancel
2. **Before each tool execution** — prevents executing the next tool in a
   multi-tool response
3. **Before `continue` to next LLM turn** — prevents sending tool results back
   to the LLM

All check `config.abort.as_ref().is_some_and(|t| t.is_cancelled())` and return
`AgentOutcome::Cancelled`.

The LLM stream itself is covered by `StreamOptions.abort` — the provider drops
the HTTP connection when the token fires mid-stream.

### Tool Bridge Pattern

Each long-running tool bridges `tokio_util::CancellationToken` (external) to
`rho_tools::cancel::CancelToken` (internal):

```rust
// Create internal CancelToken with the tool's timeout.
let ct = cancel::CancelToken::new(Some(timeout_ms));
let internal_abort = ct.emplace_abort_token();

// Bridge: when external CancellationToken fires, abort the internal CancelToken.
let external = cancel.clone();
let bridge = tokio::spawn(async move {
    external.cancelled().await;
    internal_abort.abort(cancel::AbortReason::Signal);
});

// Pass pre-built CancelToken to rho-tools function.
let result = execute_shell(options, on_chunk, ct).await;
// (or: spawn_blocking(move || grep(options, None, ct)))

bridge.abort(); // Clean up bridge if tool finished normally.
```

Fast tools (read, write, clipboard, html_to_markdown, image, process, workmux)
accept `_cancel: &CancellationToken` and ignore it.

### interactive.rs State

```rust
let mut mode = AppMode::Idle;                                // unchanged
let mut agent_generation: u64 = 0;                           // unchanged
let mut agent_cancel: Option<CancellationToken> = None;      // new
let mut bang_cancel: Option<oneshot::Sender<()>> = None;      // unchanged
```

`spawn_agent` returns a `CancellationToken`. `cancel_streaming` fires it.

`AgentEvent::Done(AgentOutcome::Cancelled)` clears `agent_cancel` and skips
auto-compaction.

### Why not `AgentHandle` struct?

The generation counter must survive after the handle is consumed on cancel (for
the event filter). Bundling them creates awkward Option gymnastics. Separate
variables are more practical.

### Why `tokio_util::CancellationToken` in the trait?

- Same type as `StreamOptions.abort` — one cancellation type in all public APIs
- `rho-agent` doesn't depend on `rho-tools` — no dependency inversion
- `tokio-util` is already a transitive dependency of `rho-agent` via `rho-ai`
- Standard tokio ecosystem type

The custom `CancelToken` stays internal to `rho-tools` where its domain-specific
features (`heartbeat()`, `deadline`, `AbortToken`) are useful.

## Files Changed

| Crate | File | Change |
|-------|------|--------|
| `rho-agent` | `Cargo.toml` | Add `tokio-util = "0.7"` |
| `rho-agent` | `agent_loop.rs` | `abort` on `AgentConfig`, 3 checkpoints, wire to `StreamOptions`, pass to `tools.execute`, construct `Cancelled` |
| `rho-agent` | `tools.rs` | `cancel: &CancellationToken` param on `Tool::execute` |
| `rho-agent` | `registry.rs` | Forward `cancel` in `ToolRegistry::execute` |
| `rho-tools` | `shell.rs` | `execute_shell` takes `CancelToken` param |
| `rho-tools` | `grep.rs` | `grep()` takes `CancelToken` param |
| `rho-tools` | `glob.rs` | `glob()` takes `CancelToken` param |
| `rho-tools` | `fd.rs` | `fuzzy_find()` takes `CancelToken` param |
| `rho` | `tools/bash.rs` | Bridge pattern, pass `CancelToken` to `execute_shell` |
| `rho` | `tools/grep.rs` | Bridge pattern, pass `CancelToken` to `grep()` |
| `rho` | `tools/find.rs` | Bridge pattern, pass `CancelToken` to `glob()` |
| `rho` | `tools/fuzzy_find.rs` | Bridge pattern, pass `CancelToken` to `fuzzy_find()` |
| `rho` | `tools/*.rs` (fast) | Add `_cancel` param, no behavior change |
| `rho` | `modes/interactive.rs` | `spawn_agent` returns token, `cancel_streaming` fires it, handle `Cancelled` |

## Verification

```bash
cargo test -p rho-agent
cargo test -p rho-tools
cargo test -p rho
cargo clippy --workspace
```
