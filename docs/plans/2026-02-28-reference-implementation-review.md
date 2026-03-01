# Reference Implementation Review: oh-my-pi & pi_agent_rust

**Date:** 2026-02-28
**Updated:** 2026-03-01
**Status:** Research complete, prioritized backlog ready (P0 tier complete)
**Scope:** Comprehensive comparison of rho's agent loop against both reference implementations to identify portable patterns and feature gaps.

## Sources

| Codebase | Language | Repository | Local Path |
|----------|----------|------------|------------|
| **oh-my-pi** | TypeScript | [can1357/oh-my-pi](https://github.com/can1357/oh-my-pi) | `.claude/code/oh-my-pi/` |
| **pi_agent_rust** | Rust | [Dicklesworthstone/pi_agent_rust](https://github.com/Dicklesworthstone/pi_agent_rust) | `.claude/code/pi-agent-rust/` |
| **rho** | Rust | this repo | `.` |

Throughout this document, source references use the format `<codebase>:<file>:<line>`.

---

## 1. Agent Loop Architecture

### 1.1 oh-my-pi: Dual-Loop with Steering & Follow-Up

oh-my-pi uses a **dual-loop** model that is the most sophisticated of the three. The `runLoop` function implements an outer loop for follow-up messages and an inner loop for tool execution + steering interrupts.

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:184-289`

```
Outer loop (follow-up messages — autonomous continuation):
  while (true):
    Inner loop (steering + tool execution):
      while (hasMoreToolCalls || pendingMessages):
        1. Inject pending steering messages into context
        2. Stream assistant response from LLM
        3. Execute tool calls (with concurrency scheduling)
        4. Check for steering messages after each tool
        5. If steering arrived, skip remaining tools
      end inner

    Check getFollowUpMessages()
    If follow-ups exist → set as pending, continue outer loop
    Else → break (agent done)
  end outer
```

**Key entry points:**
- `agentLoop()` — starts a new loop with user prompts (`oh-my-pi:packages/agent/src/agent-loop.ts:27`)
- `agentLoopContinue()` — resumes from existing context without new message (`oh-my-pi:packages/agent/src/agent-loop.ts:64`)

**Steering messages** are checked at three points:
1. Before the first turn (`oh-my-pi:packages/agent/src/agent-loop.ts:194`)
2. After each tool execution, via `checkSteering()` (`oh-my-pi:packages/agent/src/agent-loop.ts:448-467`)
3. After the inner loop completes (`oh-my-pi:packages/agent/src/agent-loop.ts:275`)

When steering arrives mid-tool-batch, remaining tools are **skipped** (result replaced with `"Skipped due to queued user message."`) and a new `AbortSignal` is fired via `steeringAbortController.abort()` (`oh-my-pi:packages/agent/src/agent-loop.ts:441-461`).

The `interruptMode` config (`"immediate"` or `"wait"`) controls whether steering checks happen between tools or only at turn boundaries (`oh-my-pi:packages/agent/src/agent-loop.ts:430`).

### 1.2 pi_agent_rust: Two-Tier Queue

pi_agent_rust mirrors the dual-loop with Rust-native concurrency:

> **Source:** `pi-agent-rust:src/agent.rs:667` (`run_loop`)

- **Steering queue:** high-priority messages checked between tool executions
- **Follow-up queue:** lower-priority messages checked when the inner loop exhausts
- Configurable `QueueMode::All` vs `QueueMode::OneAtATime`

Public entry points: `run()` at line 574, `run_with_abort()` at line 583, `run_with_content_with_abort()` at line 611.

### 1.3 rho: Single Sequential Loop with Abort Racing

rho has a single `run_agent_loop` function with no steering or follow-up message support. Cancellation is raced against every async boundary via `tokio::select!`.

> **Source:** `rho:crates/rho-agent/src/agent_loop.rs:57-342`

```
loop:
  Checkpoint: check_should_stop (cancellation + channel closure)
  Stream LLM response (raced against cancellation via tokio::select!)
  If tool calls:
    Checkpoint: check_should_stop
    Execute tools (barrier scheduling, each raced against cancellation)
    Checkpoint: check_should_stop
    continue
  Else:
    Return outcome (Stop or MaxTokens)
```

**Gap:** No mechanism for user input to arrive mid-turn. The interactive mode blocks on the agent task completing before accepting new input.

---

## 2. Tool Concurrency

### 2.1 oh-my-pi: Promise Chain Scheduler

The canonical implementation that both Rust ports derive from. Uses promise chaining to build a dependency graph:

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:562-580`

```typescript
let lastExclusive: Promise<void> = Promise.resolve();
let sharedTasks: Promise<void>[] = [];
const tasks: Promise<void>[] = [];

for (const record of records) {
    const concurrency = record.tool?.concurrency ?? "shared";
    const start = concurrency === "exclusive"
        ? Promise.all([lastExclusive, ...sharedTasks])
        : lastExclusive;
    const task = start.then(() => runTool(record, index));
    tasks.push(task);
    if (concurrency === "exclusive") {
        lastExclusive = task;
        sharedTasks = [];
    } else {
        sharedTasks.push(task);
    }
}
await Promise.allSettled(tasks);
```

The `concurrency` property is defined on the `AgentTool` interface:

> **Source:** `oh-my-pi:packages/agent/src/types.ts:228-232`

```typescript
concurrency?: "shared" | "exclusive";
```

### 2.2 pi_agent_rust: `buffer_unordered` + Barrier Flush

Uses `is_read_only()` trait method instead of `concurrency` naming. Caps parallelism at 8.

> **Source:** `pi-agent-rust:src/agent.rs:55` (constant), `pi-agent-rust:src/agent.rs:1570-1600` (parallel batch), `pi-agent-rust:src/agent.rs:1603-1809` (orchestrator)

```rust
const MAX_CONCURRENT_TOOLS: usize = 8;
```

Tool trait marker:
> **Source:** `pi-agent-rust:src/tools.rs:63-65`

```rust
fn is_read_only(&self) -> bool { false }
```

Scheduling algorithm:
1. Iterate tool calls in order
2. If `is_read_only()` → buffer for parallel batch
3. If NOT read-only → flush pending batch with `buffer_unordered(MAX_CONCURRENT_TOOLS)`, then execute sequentially
4. Race each batch against abort signal via `futures::future::select`

### 2.3 rho: Barrier Scheduling with `join_all`

Adopted oh-my-pi's `shared`/`exclusive` naming. Uses `join_all` (unbounded parallelism).

> **Source:** `rho:crates/rho-agent/src/agent_loop.rs:228-267`, `rho:crates/rho-agent/src/tools.rs:8-15`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Concurrency {
    #[default]
    Shared,
    Exclusive,
}
```

**Gaps vs pi_agent_rust:**
- No concurrency cap (all shared tools launch simultaneously)
- No steering checks between exclusive tools

---

## 3. Tool Execution

### 3.1 Streaming Tool Output

All three implementations support **incremental tool output** via callbacks during execution.

**oh-my-pi:** Tools receive an `onUpdate` callback in their execute context. The `ToolExecutionUpdate` event carries partial results to the UI.

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:531` (nonAbortable/signal handling during execution)

**pi_agent_rust:** `on_update: Option<Box<dyn Fn(ToolUpdate) + Send + Sync>>` parameter on the tool `execute` method. Bash tool streams output lines as they arrive.

> **Source:** `pi-agent-rust:src/tools.rs:50-57` (trait), `pi-agent-rust:src/tools.rs:86-89` (`ToolUpdate` struct), `pi-agent-rust:src/tools.rs:1868` (bash emit)

**rho:** `Tool::execute` accepts `Option<&OnToolUpdate>` where `OnToolUpdate = Arc<dyn Fn(&str) + Send + Sync>`. The agent loop creates per-tool callbacks via `make_update_callback()` that forward chunks to the event channel as `AgentEvent::ToolExecutionUpdate { id, content }` using non-blocking `try_send`.

> **Source:** `rho:crates/rho-agent/src/tools.rs:29` (type), `rho:crates/rho-agent/src/agent_loop.rs:381-393` (callback factory)

### 3.2 Non-Abortable Tools

oh-my-pi has a `nonAbortable` flag on `AgentTool`. When set, the tool ignores abort signals and runs to completion.

> **Source:** `oh-my-pi:packages/agent/src/types.ts:226`, `oh-my-pi:packages/agent/src/agent-loop.ts:531`

Neither pi_agent_rust nor rho has this concept.

### 3.3 Lenient Argument Validation

oh-my-pi's `lenientArgValidation` flag lets tools receive raw (unvalidated) arguments when schema validation fails, instead of returning an error to the LLM.

> **Source:** `oh-my-pi:packages/agent/src/types.ts:234`

Neither pi_agent_rust nor rho has this concept.

### 3.4 Tool Context

oh-my-pi passes per-tool-call metadata: `batchId`, `index`, `total`, and the full array of tool call info for the batch.

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:439-440`

This enables tools to be aware of their execution context (e.g., "I'm tool 3 of 5 in this batch").

---

## 4. Abort / Cancellation

### 4.1 oh-my-pi: AbortSignal + Steering Abort

Uses standard Web `AbortSignal`. When steering arrives, a secondary `AbortController` is triggered, and the two signals are combined with `AbortSignal.any()`.

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:441-444`

```typescript
const steeringAbortController = new AbortController();
const toolSignal = signal
    ? AbortSignal.any([signal, steeringAbortController.signal])
    : steeringAbortController.signal;
```

### 4.2 pi_agent_rust: Custom AbortHandle/AbortSignal

Custom implementation using `AtomicBool` + `tokio::sync::Notify` for efficient async waiting.

> **Source:** `pi-agent-rust:src/agent.rs:319-366`

```rust
pub struct AbortHandle { inner: Arc<AbortSignalInner> }
pub struct AbortSignal { inner: Arc<AbortSignalInner> }
```

Abort is raced against execution at **three granularities:**
1. Stream-level: `futures::select!(stream.next(), abort.wait())` during LLM response
2. Tool-level: individual exclusive tool raced against abort
3. Batch-level: `buffer_unordered` batch raced against abort

### 4.3 rho: CancellationToken + Racing

Uses `tokio_util::sync::CancellationToken`. Cancellation is raced via `tokio::select!` at four granularities:

1. **Stream-level:** LLM SSE stream raced against `cancel.cancelled()` (`agent_loop.rs:122-130`)
2. **Tool-level:** Individual exclusive tool execution raced against cancellation (`agent_loop.rs:267-275`)
3. **Batch-level:** Shared tool batch (`join_all`) raced against cancellation in `flush_shared_batch` (`agent_loop.rs:364-372`)
4. **Retry-level:** Backoff delay raced against cancellation (`agent_loop.rs:156-161`)

Additionally, 3 discrete `check_should_stop` checkpoints guard turn boundaries.

> **Source:** `rho:crates/rho-agent/src/agent_loop.rs:397-412` (`check_should_stop`)

---

## 5. Intent Tracing

Unique to oh-my-pi. Injects a `_i` (intent) field into every tool's JSON schema as a required parameter. The model fills it with its reasoning for the tool call. The field is stripped before execution and stored separately for debugging/auditing.

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:126-179`

```typescript
export const INTENT_FIELD = "_i";

function injectIntentIntoSchema(schema: unknown): unknown {
    // Adds _i as first property, marks required
}

function extractIntent(args: Record<string, unknown>): {
    intent?: string;
    strippedArgs: Record<string, unknown>;
} {
    const intent = args[INTENT_FIELD];
    const { [INTENT_FIELD]: _ignored, ...strippedArgs } = args;
    return { intent: typeof intent === "string" ? intent : undefined, strippedArgs };
}
```

Usage during tool execution:
> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:488`

**Cost:** Minimal token overhead per tool call. High value for debugging and understanding LLM decisions.

**Neither pi_agent_rust nor rho has this.**

---

## 6. Delta Throttling

oh-my-pi batches consecutive text/thinking/tool-call delta events to limit UI updates to 20/sec. Deltas for the same content index and type are merged. Non-delta events flush immediately.

> **Source:** `oh-my-pi:packages/ai/src/utils/event-stream.ts:95-164`

```typescript
readonly #throttleMs = 50; // 20 updates/sec

// Delta events get batched and throttled
// Flush on: non-delta event OR throttle window expiry
```

**Neither pi_agent_rust nor rho has this.** rho forwards every `TextDelta` and `ThinkingDelta` individually from the SSE stream to the UI.

---

## 7. Edit Tool

### 7.1 oh-my-pi: Three Edit Modes

Dynamically selects edit strategy per model:

> **Source:** `oh-my-pi:packages/coding-agent/src/patch/index.ts`

| Mode | Description | Best For |
|------|-------------|----------|
| **Replace** | `old_text` / `new_text` with fuzzy matching | Most models (default) |
| **Patch** | Unified diff hunks (`op: "create" \| "delete" \| "update"`) | Models good at diff format |
| **Hashline** | Line-addressed via content hashes (`LINE#HASH:CONTENT`) | Large files, resilience |

Hashline implementation:
> **Source:** `oh-my-pi:packages/coding-agent/src/patch/hashline.ts`

Special features across all modes:
- Fuzzy matching with configurable threshold (`oh-my-pi:packages/coding-agent/src/patch/shared.ts`)
- Line ending detection/preservation (CRLF vs LF)
- BOM (Byte Order Mark) preservation
- Unicode NFC/NFD normalization variants for matching
- Atomic write via temp file + rename
- 100MB file size limit
- Jupyter notebook rejection (redirects to NotebookEdit tool)

### 7.2 pi_agent_rust: Two Edit Modes

Standard replace + hashline. Same fuzzy matching and Unicode normalization.

> **Source:** `pi-agent-rust:src/tools.rs` (edit tool implementations around line 2077+)

### 7.3 rho: Replace Mode with Fuzzy Matching

rho has an `EditTool` implementing replace mode (`old_string` / `new_string`) with fuzzy matching:

- **Exact match** fast path, then normalized match (trailing whitespace stripped, Unicode spaces/quotes/dashes normalized), then NFC/NFD variants
- Line ending detection/preservation (CRLF vs LF)
- BOM (Byte Order Mark) preservation
- Atomic write via temp file + rename
- Ambiguity rejection (multiple matches → error)
- Contextual diff output via `similar` crate

> **Source:** `rho:crates/rho/src/tools/edit.rs`

**Remaining gap vs references:** No hashline or patch edit modes.

---

## 8. Context Management

### 8.1 oh-my-pi: Hook-Based Context Transformation

Two hooks for context manipulation before LLM calls:

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts:308-313`

```typescript
if (config.transformContext) {
    messages = await config.transformContext(messages, signal);
}
const llmMessages = await config.convertToLlm(messages);
```

- `transformContext` — operates on `AgentMessage[]` (high-level). Used for pruning, token accounting, injecting external context.
- `convertToLlm` — converts to provider-specific format. Filters to `user`/`assistant`/`toolResult` roles.

### 8.2 pi_agent_rust: Cow-Based Zero-Copy

Uses `Cow<'a, [Message]>` and `Cow<'a, str>` in `Context` to avoid deep-cloning message history.

> **Source:** `pi-agent-rust:src/agent.rs:49` (Cow usage)

### 8.3 rho: mem::take Swap + Arc

Uses `std::mem::take` to swap message vecs out/back without allocation. System prompt is `Arc<String>`.

> **Source:** `rho:crates/rho-agent/src/agent_loop.rs` (lines around 97-106)

**Gap:** No `transformContext` hook for external context manipulation.

---

## 9. Compaction / Summarization

### 9.1 oh-my-pi: Hook-Based Custom Compaction

Compaction is exposed as a hook (`session_before_compact`). Extensions can replace the default summarization strategy entirely.

> **Source:** `oh-my-pi:packages/coding-agent/src/extensibility/hooks/types.ts`

Example: using Gemini Flash (cheaper model) for summarization instead of the main model.

### 9.2 pi_agent_rust: Two-Phase Background Compaction with File Tracking

> **Source:** `pi-agent-rust:src/compaction.rs:1-99` (orchestrator), `pi-agent-rust:src/compaction.rs:116-142` (file ops tracking)

Features:
- **Two-phase:** background worker prepares summary, main thread applies it
- **File operation tracking:** builds `FileOperations` struct tracking read/written/edited files for better summaries
- **Iterative summarization:** can update prior summaries incrementally
- **Token estimation:** `CHARS_PER_TOKEN_ESTIMATE = 3` (conservative for code)
- **Settings:** configurable `context_window_tokens`, `reserve_tokens` (~8%), `keep_recent_tokens` (~10%)

### 9.3 rho: Foreground Summarization with File Tracking

Compaction with LLM-generated summary and file operation tracking. `FileOperations` struct extracts read/written/edited files from tool call blocks and includes them in the summarization context. Supports iterative summarization (updating prior summaries). No background worker, no custom hooks.

> **Source:** `rho:crates/rho/src/compaction/compact.rs`, `rho:crates/rho/src/compaction/file_ops.rs`

**Remaining gaps:** No background compaction worker, no custom hooks.

---

## 10. Event System

### 10.1 oh-my-pi Events

> **Source:** `oh-my-pi:packages/agent/src/agent-loop.ts` (events emitted throughout)

```typescript
type AgentEvent =
    | { type: "agent_start" }
    | { type: "agent_end"; messages: AgentMessage[] }
    | { type: "turn_start" }
    | { type: "turn_end"; message: AssistantMessage; toolResults: ToolResultMessage[] }
    | { type: "message_start"; message: AgentMessage }
    | { type: "message_update"; message: AssistantMessage; event: AssistantMessageEvent }
    | { type: "message_end"; message: AgentMessage }
    | { type: "tool_execution_start"; toolCallId; toolName; args }
    | { type: "tool_execution_update"; toolCallId; toolName; args; partialResult }
    | { type: "tool_execution_end"; toolCallId; toolName; result; isError }
```

### 10.2 pi_agent_rust Events

> **Source:** `pi-agent-rust:src/agent.rs:205-311`

Superset of oh-my-pi with additional events:
- `AutoCompactionStart { reason }` / `AutoCompactionEnd { result, aborted, will_retry, error }`
- `AutoRetryStart { attempt, max_attempts, delay_ms, error_message }` / `AutoRetryEnd { success, attempt, final_error }`
- `ExtensionError { extension_id, event, error }`

### 10.3 rho Events

> **Source:** `rho:crates/rho-agent/src/events.rs:7-28`

```rust
pub enum AgentEvent {
    TurnStart { turn: u32 },
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
    ToolExecutionUpdate { id: String, content: String },
    ToolCallResult { id: String, is_error: bool },
    MessageComplete(AssistantMessage),
    ToolResultComplete { tool_use_id: String, content: Arc<String>, is_error: bool },
    RetryScheduled { attempt: u32, delay_ms: u64, error: String },
    Done(AgentOutcome),
}
```

**Gaps:**
- No `AgentStart` / `AgentEnd` lifecycle events
- No compaction events
- No `TurnEnd` with combined message + tool results

---

## 11. Extension / Hook System

### 11.1 oh-my-pi

Hook-based architecture with lifecycle events:

> **Source:** `oh-my-pi:packages/coding-agent/src/extensibility/hooks/types.ts`

```typescript
pi.on("session_before_compact", async (event, ctx) => { ... })
pi.on("session_start", async (_event, ctx) => { ... })
pi.on("session_tree", async (_event, ctx) => { ... })
```

Extension API:
- `registerCommand(name, { description, handler })` — slash commands
- `setActiveTools(names)` — enable/disable tools dynamically
- `appendEntry(type, data)` — persistent custom data in session
- MCP server integration (stdio, SSE, HTTP transports with OAuth)

### 11.2 pi_agent_rust

Full extension system with multiple runtimes:

> **Source:** `pi-agent-rust:src/extensions.rs`

- **Runtimes:** JS (QuickJS), Native Rust, WASM (optional)
- **Permission system:** `ExtensionPolicy` with modes (Strict, Prompt, Permissive)
- **Risk tiers:** Commands classified as Low/Medium/High/Critical
- **Tool hooking:** `dispatch_tool_call_hook` (pre-execution blocking), `dispatch_tool_result_hook` (post-execution modification)
- **Fail-open:** Errors in hooks don't fail the agent

### 11.3 rho

No extension or hook system.

---

## 12. Sub-Agents

### 12.1 oh-my-pi

Embedded agent definitions with frontmatter configuration:

> **Source:** `oh-my-pi:packages/coding-agent/src/capability/skill.ts:10-31`

```typescript
// Embedded agents:
{ name: "explore",  template: exploreMd }
{ name: "plan",     template: planMd }
{ name: "designer", template: designerMd }
{ name: "reviewer", template: reviewerMd }
{ name: "task",     spawns: "*", model: "default", thinkingLevel: "medium" }
{ name: "quick_task", thinkingLevel: "minimal" }
```

Frontmatter fields: `name`, `description`, `tools[]`, `spawns` (which agents can spawn this), `model`, `thinkingLevel`, `blocking` (wait for completion).

**Handoff pattern:** summarize current conversation with LLM, create child session with parent tracking, generated prompt appears in editor for review.

### 12.2 pi_agent_rust & rho

Neither has sub-agent support.

---

## 13. Session Persistence

All three use JSONL-based session files with snowflake IDs and breadcrumb files for discovery.

**oh-my-pi** supports: Message, ModelChange, ThinkingLevelChange, Compaction, BranchSummary, Label, Custom entry types. Extensions can append custom persistent data via `appendEntry()`.

**pi_agent_rust** supports the same entry types plus diagnostics and orphan entry handling.
> **Source:** `pi-agent-rust:src/session.rs:34-126`

**rho** supports: Message, ModelChange, ThinkingLevelChange, Compaction, BranchSummary entry types.
> **Source:** `rho:crates/rho/src/session/mod.rs:45-126`

**Gap:** No custom entry types or extension-driven persistence in rho.

---

## Unified Comparison Matrix

| Feature | oh-my-pi | pi_agent_rust | rho | Status |
|---------|----------|---------------|-----|--------|
| Tool concurrency (shared/exclusive) | Promise chain | `buffer_unordered(8)` | `join_all` | **Done** |
| Streaming tool output | `onUpdate` callback | `on_update` callback | `OnToolUpdate` + `ToolExecutionUpdate` event | **Done** |
| Abort racing (stream/tool/batch) | `AbortSignal.any()` | `futures::select!` at 3 levels | `tokio::select!` at 4 levels | **Done** |
| Edit tool (replace mode) | 3 modes (replace/patch/hashline) | 2 modes (replace/hashline) | Replace mode + fuzzy matching | **Done** |
| Fuzzy matching (edit) | Yes + Unicode NFC/NFD | Yes + Unicode NFC/NFD | Yes + Unicode NFC/NFD | **Done** |
| File tracking in compaction | Via extensions | Built-in `FileOperations` | Built-in `FileOperations` | **Done** |
| Channel error handling | N/A (in-process) | N/A | `check_should_stop` | **Done** |
| Steering messages (mid-turn interrupts) | Dual-loop + `getSteeringMessages` | Two-tier queue | None | Gap |
| Follow-up messages (autonomous continuation) | Outer loop + `getFollowUpMessages` | Follow-up queue | None | Gap |
| Intent tracing | `_i` field injection | None | None | Gap |
| Delta throttling | 50ms batch + merge | None | None | Gap |
| Edit tool (hashline/patch modes) | Patch + hashline | Hashline | None | Gap |
| Context transform hooks | `transformContext` + `convertToLlm` | None | None | Gap |
| Background compaction | Via hooks | Two-phase worker | Foreground | Gap |
| Extension/hook system | Lifecycle hooks + MCP | JS/Native/WASM + permissions | None | Gap |
| Sub-agents | Embedded defs + handoff | None | None | Gap |
| Non-abortable tools | `nonAbortable` flag | None | None | Gap |
| Lenient arg validation | `lenientArgValidation` | None | None | Gap |
| Tool batch metadata | `batchId`, `index`, `total` | None | None | Gap |
| Concurrency cap | Unbounded (Promise.all) | `MAX_CONCURRENT_TOOLS = 8` | Unbounded (`join_all`) | Gap |
| Lifecycle events (`TurnEnd`) | `turn_end` with message + results | Full lifecycle | `TurnStart` + `Done` only | Gap |

---

## Prioritized Backlog

### Completed

| ID | Feature | Commit |
|----|---------|--------|
| ~~P0-1~~ | **Streaming tool output** (`ToolExecutionUpdate` event + `OnToolUpdate` callback) | `bdba576` |
| ~~P0-2~~ | **Abort racing on tool execution** (`tokio::select!` at 4 levels) | `e1be95d` |
| ~~P0-3~~ | **Edit tool** (replace mode + fuzzy matching, BOM/CRLF, Unicode NFC/NFD) | `9f7f8af` |
| ~~P1-4~~ | **File operation tracking in compaction** (`FileOperations` struct) | (integrated into compaction module) |

### P1 — Clear value, moderate effort

| ID | Feature | Effort | Impact | Reference |
|----|---------|--------|--------|-----------|
| P1-1 | **Concurrency cap** (`buffer_unordered(8)`) | Trivial | Defensive limit on parallel tools | `pi-agent-rust:src/agent.rs:55` |
| P1-2 | **Intent tracing** (`_i` field injection) | Small | Debugging/auditing LLM tool decisions | `oh-my-pi:agent-loop.ts:126-179` |
| P1-3 | **Delta throttling** (50ms batch + merge) | Small | Reduces UI render pressure under fast SSE | `oh-my-pi:event-stream.ts:95-164` |
| P1-5 | **`TurnEnd` event** (combined message + tool results) | Small | Cleaner turn boundaries for steering groundwork | `pi-agent-rust:src/agent.rs:205-311` |

### P2 — Significant refactoring

| ID | Feature | Effort | Impact | Reference |
|----|---------|--------|--------|-----------|
| P2-1 | **Steering messages** (mid-turn user input) | Large | User can redirect agent during tool execution | `oh-my-pi:agent-loop.ts:194-275,448-467` |
| P2-2 | **Follow-up message loop** (autonomous continuation) | Medium | Agent continues without user interaction | `oh-my-pi:agent-loop.ts:280-289` |
| P2-3 | **Pre/post tool execution hooks** | Medium | Foundation for extensions | `pi-agent-rust:src/agent.rs` (dispatch_tool_call_hook) |
| P2-4 | **Hashline edit mode** | Medium | Resilient edits on large files | `oh-my-pi:packages/coding-agent/src/patch/hashline.ts` |
| P2-5 | **Background compaction worker** | Medium | Non-blocking summarization | `pi-agent-rust:src/compaction.rs:1-99` |

### P3 — Long-term / large scope

| ID | Feature | Effort | Impact | Reference |
|----|---------|--------|--------|-----------|
| P3-1 | **Sub-agent definitions + spawn** | Large | Task delegation, specialization | `oh-my-pi:packages/coding-agent/src/capability/skill.ts:10-31` |
| P3-2 | **Extension/MCP system** | Very large | Ecosystem play, third-party tools | `pi-agent-rust:src/extensions.rs` |
| P3-3 | **Context transform hooks** | Medium | Pluggable context manipulation | `oh-my-pi:agent-loop.ts:308-313` |
| P3-4 | **Permission system** (risk tiers) | Large | Tool approval for sensitive ops | `pi-agent-rust:src/extensions.rs` (ExtensionPolicy) |

---

## Implementation Notes

### P1-1: Concurrency Cap

Replace `join_all` with `futures::stream::iter(futures).buffer_unordered(8).collect()` in `flush_shared_batch`. One-line change.
