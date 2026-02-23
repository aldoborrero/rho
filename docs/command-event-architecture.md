# Command & event architecture

This document describes the target architecture for user input handling, command dispatch, and event processing in the Rust binary (`rho`). The design evolves in three phases, each a standalone PR that doesn't break prior work.

## Current state

All command logic lives in `commands/mod.rs` (867 lines). Commands return a `CommandResult` enum with 4 variants (`Message`, `Exit`, `NewSession`, `ChangeDir`). The event loop in `modes/interactive.rs` owns all mutable state and pattern-matches on the result to perform side effects. Input routing (slash commands → bang commands → normal prompt) is inlined in the `EditorSubmit` match arm (~80 lines of nested if/else).

### Architecture principle: Decide → Declare → Apply

```
User input (text)
    ↓
  classify (pure)        → InputAction enum
    ↓
  decide (reads state)   → CommandResult / Effect enum
    ↓
  apply (owns state)     → mutations to session, app, terminal
    ↓
  render                 → draw frame
```

Each layer does one thing. Classification doesn't read session. Decision reads but doesn't mutate. Application mutates but doesn't decide. This avoids the JS pattern of passing a god object (`InteractiveModeContext`) and having commands perform arbitrary mutations — which in Rust would mean `Arc<Mutex<>>` or `RefCell` soup to satisfy the borrow checker.

---

## Phase 1: Command module split

**Goal:** Split the monolithic `commands/mod.rs` into domain-grouped files, introduce `CommandContext`, expand `CommandResult` with new effect variants.

### File structure

```
commands/
    mod.rs              thin re-export hub
    types.rs            CommandResult, CommandContext, SlashCommand, SubcommandDef
    registry.rs         COMMANDS array, parse_command()
    dispatch.rs         execute_command(), execute_bang()
    handlers/
        mod.rs          re-exports
        help.rs         /help, /hotkeys
        session.rs      /session, /new, /debug, /export, /fork
        clipboard.rs    /copy, /dump, extract_assistant_text()
        navigation.rs   /move
        model.rs        /model, /usage
        compact.rs      /compact (returns Compact variant)
        plan.rs         /plan (stub)
```

### Types

```rust
// commands/types.rs

/// Result of executing a slash command.
/// Each variant declares intent — the event loop performs the effect.
pub enum CommandResult {
    /// Display a message in chat (not sent to AI).
    Message(String),
    /// Exit the application.
    Exit,
    /// Clear chat and start a new session.
    NewSession,
    /// Change working directory.
    ChangeDir(String),
    /// Fork the current session. Event loop calls session.fork().
    Fork,
    /// Trigger conversation compaction (optional focus instructions).
    Compact(Option<String>),
    /// Change the active model.
    ModelChange(String),
    /// No visible output (e.g., clipboard operation already done).
    Silent,
}

/// Subcommand metadata (for autocomplete, not dispatch).
pub struct SubcommandDef {
    pub name: &'static str,
    pub description: &'static str,
}

/// Metadata for a registered slash command.
pub struct SlashCommand {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args_hint: Option<&'static str>,
    pub subcommands: &'static [SubcommandDef],
}

/// Read-only context provided to slash command handlers.
/// Borrows from the event loop's owned state. Commands read but never mutate.
pub struct CommandContext<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub session: &'a SessionManager,
    pub config: &'a Config,
    pub model: &'a str,
    pub tools: &'a ToolRegistry,
}
```

### Dispatch

Flat `match` on `ctx.name`. No trait objects, no dynamic dispatch, no `HashMap<&str, Box<dyn Handler>>`. A match with 28 arms compiles to a jump table and handles mixed sync/async handlers without a uniform trait.

```rust
// commands/dispatch.rs

pub async fn execute_command(ctx: &CommandContext<'_>) -> anyhow::Result<CommandResult> {
    match ctx.name {
        "help"    => Ok(handlers::help::cmd_help()),
        "exit"    => Ok(CommandResult::Exit),
        "new"     => Ok(CommandResult::NewSession),
        "model"   => Ok(handlers::model::cmd_model(ctx)),
        "session" => Ok(handlers::session::cmd_session(ctx)),
        "copy"    => handlers::clipboard::cmd_copy(ctx).await,
        "dump"    => handlers::clipboard::cmd_dump(ctx).await,
        "usage"   => Ok(handlers::model::cmd_usage()),
        "hotkeys" => Ok(handlers::help::cmd_hotkeys()),
        "move"    => handlers::navigation::cmd_move(ctx),
        "compact" => Ok(handlers::compact::cmd_compact(ctx)),
        "plan"    => Ok(handlers::plan::cmd_plan()),
        "export"  => Ok(handlers::session::cmd_export()),
        "debug"   => Ok(handlers::session::cmd_debug(ctx)),
        "fork"    => Ok(handlers::session::cmd_fork()),
        _ => Ok(CommandResult::Message(format!(
            "Unknown command: /{}. Type /help for available commands.", ctx.name
        ))),
    }
}
```

### Subcommand pattern

Subcommands route inside the handler function. No framework needed.

```rust
// Example: future /mcp handler
pub async fn cmd_mcp(ctx: &CommandContext<'_>) -> anyhow::Result<CommandResult> {
    let (subcmd, sub_args) = match ctx.args.split_once(char::is_whitespace) {
        Some((s, a)) => (s.trim(), a.trim()),
        None => (ctx.args.trim(), ""),
    };
    match subcmd {
        "" | "help" => Ok(CommandResult::Message(mcp_help())),
        "list"      => cmd_mcp_list(ctx).await,
        "add"       => cmd_mcp_add(sub_args, ctx).await,
        _           => Ok(CommandResult::Message(format!("Unknown: /mcp {subcmd}"))),
    }
}
```

`SubcommandDef` in `SlashCommand` is metadata for autocomplete only.

### Event loop changes (modes/interactive.rs)

Build `CommandContext` at the dispatch site. Add match arms for new `CommandResult` variants:

```rust
// Build context
let cmd_ctx = commands::CommandContext {
    name: cmd_name,
    args,
    session: &session,
    config: &config,
    model: &cli.model,
    tools: &tools,
};
match commands::execute_command(&cmd_ctx).await? {
    CommandResult::Message(msg) => { /* add to chat */ }
    CommandResult::Exit => break,
    CommandResult::NewSession => { session.clear().await?; app.chat.clear(); }
    CommandResult::ChangeDir(path) => { std::env::set_current_dir(&path)?; /* msg */ }
    CommandResult::Fork => {
        match session.fork() {
            Ok(_) => { /* show "Forked: {id}", update status */ }
            Err(e) => { /* show error */ }
        }
    }
    CommandResult::Compact(instructions) => {
        // Stub — will be wired when compaction is implemented.
        let _ = instructions;
        /* show "not yet implemented" */
    }
    CommandResult::ModelChange(model) => {
        let _ = model;
        /* show "not yet implemented" */
    }
    CommandResult::Silent => {}
}
```

### Testing

Each handler file gets `#[cfg(test)] mod tests`. Test helper:

```rust
fn test_ctx<'a>(args: &'a str, session: &'a SessionManager) -> CommandContext<'a> {
    CommandContext {
        name: "",
        args,
        session,
        config: &Config { api_key: String::new(), base_url: String::new(), is_oauth: false },
        model: "test-model",
        tools: &ToolRegistry::new(),
    }
}
```

---

## Phase 2: Input routing

**Goal:** Extract input classification from the event loop into a testable `route_input()` function that returns an `InputAction` enum.

### Types

```rust
// modes/input.rs

/// What the input router decided to do.
pub enum InputAction<'a> {
    /// A recognized slash command.
    SlashCommand { name: &'static str, args: &'a str },
    /// A `/`-prefixed input that didn't match any registered command.
    UnknownCommand(&'a str),
    /// A `!`-prefixed shell command.
    BangCommand(&'a str),
    /// Normal message to send to the agent.
    UserMessage(&'a str),
    /// Empty input, ignore.
    Empty,
}
```

### Router function

```rust
pub fn route_input(text: &str) -> InputAction<'_> {
    let text = text.trim();
    if text.is_empty() {
        return InputAction::Empty;
    }
    if text.starts_with('/') {
        return match crate::commands::parse_command(text) {
            Some((name, args)) => InputAction::SlashCommand { name, args },
            None => InputAction::UnknownCommand(
                text.split_whitespace().next().unwrap_or(text)
            ),
        };
    }
    if text.starts_with('!') && !text.starts_with("!!") {
        return InputAction::BangCommand(&text[1..]);
    }
    InputAction::UserMessage(text)
}
```

**Design note:** `route_input` is a pure classifier — it reads no state and performs no side effects. The event loop builds `CommandContext` and dispatches after classification.

### Event loop after Phase 2

The `EditorSubmit` handler uses `route_input` → dispatch → `apply_command_result`:

```rust
match route_input(&text) {
    InputAction::Empty => {}
    InputAction::SlashCommand { name, args } => {
        let ctx = commands::CommandContext { name, args, session: &session, ... };
        let result = commands::execute_command(&ctx).await?;
        if matches!(apply_command_result(result, ...)?, ApplyOutcome::Exit) { break; }
    }
    InputAction::UnknownCommand(cmd) => { show_chat_message(...); }
    InputAction::BangCommand(cmd) => {
        let output = commands::execute_bang(cmd, &tools).await?;
        show_chat_message(..., output);
    }
    InputAction::UserMessage(text) => { /* append to session, spawn agent */ }
}
```

### What this unlocks

- Adding new input types (python, skills, steer) = add variant + classifier check
- Input classification is testable: `assert!(matches!(route_input("!ls"), InputAction::BangCommand(_)))`
- Event loop `EditorSubmit` arm goes from ~80 lines to ~30

---

## Phase 3: Mode state machine

**Goal:** Replace boolean flags (`is_streaming`, future `is_compacting`, `plan_mode_enabled`, etc.) with a single `AppMode` enum. The event loop becomes a state machine where behavior depends on `(mode, event)`.

### Types

```rust
// modes/state.rs

/// Application mode — only one active at a time.
/// Prevents impossible states (e.g., compacting while streaming).
pub enum AppMode {
    /// Waiting for user input.
    Idle,
    /// Agent is running, streaming tokens.
    Streaming,
    /// Compaction LLM call in progress.
    Compacting,
    /// Plan mode — agent writes to plan file, user approves.
    PlanMode { file_path: String },
    /// Interactive selector UI is open.
    Selecting(SelectorKind),
}

pub enum SelectorKind {
    SessionPicker,
    BranchTree,
    ModelPicker,
    Settings,
}
```

### Migration from boolean flags

| Current flag | Becomes |
|---|---|
| `is_streaming: bool` | `AppMode::Streaming` |
| (future) `is_compacting` | `AppMode::Compacting` |
| (future) `plan_mode_enabled` | `AppMode::PlanMode { .. }` |
| (future) `showing_selector` | `AppMode::Selecting(..)` |

---

## Summary

| Phase | Deliverable | Unlocks |
|---|---|---|
| 1 | Command split + `CommandContext` + expanded `CommandResult` | Clean organization, `/fork` wired, subcommand support |
| 2 | `route_input() → InputAction` | Testable routing, easy new input types (skills, python) |
| 3 | `AppMode` state machine | Mode safety, streaming/compacting/plan mode without flag soup |

Each phase is a standalone PR. Phase 1 is prerequisite for 2. Phase 2 is prerequisite for 3. No phase requires rewriting prior work.
