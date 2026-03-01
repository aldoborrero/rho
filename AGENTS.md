# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project Overview

Rho is a terminal-based AI coding agent written in Rust (edition 2024). It provides an interactive TUI for conversing with LLMs (Anthropic, OpenAI) and executing tools (bash, file operations, grep, etc.) autonomously via an agent loop. The binary is `rho`.

## Nix & Development Environment

The project uses **Nix flakes** for fully reproducible development environments. The Rust nightly toolchain (required for edition 2024) is provided via [fenix](https://github.com/nix-community/fenix), and the Nix build uses [crane](https://github.com/ipetkov/crane) for Rust packaging. The flake is organized with [numtide/blueprint](https://github.com/numtide/blueprint) under `nix/`.

**direnv integration**: The repository includes an `.envrc` that calls `use flake` to automatically activate the Nix dev shell when entering the directory. It also adds `bin/` to PATH and sources `.envrc.local` if present (for local overrides like API keys). With `direnv allow`, all tools (Rust nightly, bun, pkg-config, git, and on Linux `libx11` for clipboard) are available without manual `nix develop`.

```bash
# Automatic (recommended) — direnv loads the flake dev shell on cd
direnv allow

# Manual — enter the dev shell explicitly
nix develop

# Build (debug)
cargo build

# Build (release, with LTO + strip)
cargo build --release

# Run
cargo run -p rho -- [args]
```

Key libraries: Rust edition 2024 (nightly), `tokio` (multi-threaded runtime), `async-trait`, `serde`/`serde_json`. See **Validation Matrix** for test, clippy, and format commands.

## Architecture

### Workspace Crates

The workspace has 6 first-party crates plus 2 vendored dependencies:

- **`rho`** — Binary crate. Entry point, CLI (clap), TUI app, config resolution, tool registration, interactive mode event loop, session management, slash commands, and Anthropic model setup. This is the "application shell" that wires everything together.
- **`rho-agent`** — Agent loop engine. Runs the autonomous LLM→tool→LLM cycle. Streams responses via `rho-ai`, executes tools from a `ToolRegistry`, handles retries, and emits `AgentEvent`s over an mpsc channel. Provider-agnostic — depends on `rho-ai` for the LLM layer.
- **`rho-ai`** — LLM provider abstraction. Defines the `Provider` trait, `Model`/`ModelRegistry`, streaming (`stream`/`complete`), SSE parsing, retry logic, and type conversions. Has concrete providers for Anthropic Messages API, OpenAI Completions, and OpenAI Responses.
- **`rho-tools`** — Pure Rust tool implementations extracted from the legacy N-API codebase. Provides clipboard, file discovery (`fd`), grep, glob, HTML-to-markdown, image processing, shell/PTY execution, process management, profiling, and workmux integration. No N-API dependencies.
- **`rho-tui`** — Terminal UI framework built on crossterm. Component model (`Component` trait), editor with vim-like motions, markdown rendering with syntax highlighting (syntect), keybindings, fuzzy matching, overlays, and theme system.
- **`rho-text`** — ANSI-aware text utilities. Width measurement, slicing, wrapping, truncation, and sanitization. Both UTF-8 and UTF-16 APIs. Zero-allocation fast paths for ASCII.
- **`brush-core`** / **`brush-builtins`** — Vendored forks of the [brush shell](https://github.com/reubeno/brush) crates, patched via `[patch.crates-io]` in the root `Cargo.toml`. These are excluded from the workspace members but included via path dependencies.

### Key Data Flow

1. User input → `rho` interactive mode event loop
2. User message → `rho-agent::agent_loop::run_agent_loop()` spawned as tokio task
3. Agent loop calls `rho-ai::stream()` → SSE stream from LLM provider
4. LLM response with tool calls → `ToolRegistry::execute()` → `rho-tools` implementations
5. Tool results fed back to LLM (loop continues until `end_turn` or `max_tokens`)
6. All events emitted via `AgentEvent` mpsc channel → TUI renders in real-time

### Tool System

Tools implement `rho_agent::tools::Tool` trait (`name`, `description`, `input_schema`, `execute`). Registered in `rho/src/tools/mod.rs::create_default_registry()`. Built-in tools: bash, read, write, grep, find, fuzzy_find, clipboard, html_to_markdown, process, image, workmux.

### Configuration

API key resolution order: `--api-key` flag → `ANTHROPIC_API_KEY` env → `ANTHROPIC_OAUTH_TOKEN` env → `~/.rho/config/config.json`. Base URL overridable via `ANTHROPIC_BASE_URL`.

### Session Persistence

Sessions stored as JSON files under `.rho/` in the working directory. Supports create, resume (`--resume`), continue most recent (`--continue`), and ephemeral (`--no-session`). Uses snowflake IDs and breadcrumb files for session discovery.

## Agent Workflow

1. **Read before write** — inspect the existing module, adjacent tests, and trait boundaries before editing.
2. **Define scope** — one concern per change; avoid mixed feature+refactor+infra patches.
3. **Implement minimal patch** — apply the engineering principles below; do not over-engineer.
4. **Validate** — run the commands in the Validation Matrix for the change scope.
5. **Document impact** — update comments/docs for behavior changes, risk, and side effects.

## Engineering Principles

### Minimize allocation, prove it with benchmarks

Prefer borrowing over cloning. Use `&str` instead of `String`, `&Value` instead of owned `Value` in function signatures where ownership isn't needed. Use `Arc` for large shared data (2KB+), but not for small copyable values — atomic refcount ops are slower than memcpy for strings under ~256 bytes. When optimizing, benchmark first with criterion; intuition about allocation cost is often wrong.

- Do not wrap small values in `Arc` for "performance" — benchmark first.
- Do not clone `serde_json::Value` to pass to functions that only read it — pass `&Value`.

### Respect crate boundaries

Dependencies flow inward: `rho` → `rho-agent` → `rho-ai`. Never reverse this direction. `rho-agent` must not depend on `rho-tui`. `rho-ai` must not know about agent concepts (tool registry, events). Tool implementations in `rho/src/tools/` must not import `rho-tui`. The `rho-tools` crate is a leaf — it depends on nothing in the workspace.

- Do not import between sibling crates. Only `rho` (the binary) wires everything together.
- Do not add `tokio` features to `rho-agent`'s `Cargo.toml` — it gets them transitively through `rho`. Adding them directly creates hidden feature unification bugs.

### Keep the tool trait surface minimal

`Tool::execute` receives `&serde_json::Value` and returns `ToolOutput`. Tools must not hold mutable state, access the agent loop, or know about the TUI. All side effects (file writes, shell execution) happen inside `execute` and are reported through the return value or the `on_update` streaming callback.

- Do not make tools stateful. Each `execute` call is independent.

### Fail explicitly at boundaries

Use `anyhow` for application-level errors in `rho`. Use `thiserror` for typed errors in library crates (`rho-ai`, `rho-tools`). Tools return `anyhow::Result<ToolOutput>` — errors become tool error results shown to the LLM, not panics. Provider errors surface as `StreamEvent::Error` with retryable/non-retryable classification.

- Do not use `unwrap()` in production code paths. Use `?` or `anyhow::bail!`. `unwrap()` is acceptable in tests and benchmarks only.

### Session format is a compatibility contract

Session JSONL files use `camelCase` field names and TypeScript-compatible type discriminators for cross-tool interoperability. Do not rename fields, change enum tag values, or restructure entry types without a migration path. New entry types can be added freely — unknown types are skipped on read.

- Do not rename session JSONL fields. Add new fields instead.

### Do not modify vendored crates

`brush-core` and `brush-builtins` are vendored via `[patch.crates-io]`. Do not change their lint configurations, module structure, or public API. If upstream changes are needed, apply targeted patches and document the diff.

- Do not silence clippy with `#[allow]` on vendored crates. Workspace lints are excluded from them via the `exclude` list in root `Cargo.toml`.
- Do not add edition 2024 features that break `cargo test -p <crate>` in isolation.

## Risk Tiers

Classify changes by blast radius to calibrate review depth. When uncertain, treat as one tier higher.

**High risk** — changes here cascade across crates or touch security-sensitive surfaces:

- `rho-agent/src/agent_loop.rs` — core orchestration; affects every LLM turn, tool dispatch, cancellation, and retry
- `rho-agent/src/tools.rs`, `registry.rs`, `events.rs`, `types.rs` — cross-crate trait boundaries; signature changes cascade to all tool implementations and TUI consumers
- `rho-ai/src/types.rs` — shared message types used by every provider, the agent loop, and session persistence; structural changes break serialization compat
- `rho-ai/src/providers/*.rs` — network I/O, API key handling, SSE parsing; auth or request errors silently break all LLM communication
- `rho/src/tools/bash.rs`, `rho-tools/src/shell.rs` — arbitrary shell execution with streaming output; highest security surface
- `rho/src/session/` — user data persistence; format changes can corrupt or orphan existing sessions
- `rho/src/settings.rs` — API key resolution chain, model defaults; misconfiguration silently degrades the app

**Medium risk** — contained impact but can cause subtle bugs:

- `rho/src/tools/*.rs` (non-bash) — individual tool implementations; a bug affects one tool, not the system
- `rho-ai/src/stream.rs`, `events.rs`, `retry.rs`, `transform.rs` — streaming infrastructure; bugs surface as garbled output or dropped messages
- `rho-agent/src/convert.rs` — agent ↔ rho-ai type mapping; incorrect conversion silently corrupts message history sent to the LLM
- `rho/src/modes/interactive.rs` — main event loop and state machine; bugs cause hangs or missed events
- `rho/src/compaction/` — context summarization; bugs degrade quality but don't corrupt state
- `rho/src/prompts/` — system prompt assembly; affects agent behavior but is easily reversible
- `rho-tui/src/components/editor/` — editor state machine, vim motions, undo/redo

**Low risk** — self-contained, no side effects or data mutations:

- `rho-text/src/` — pure text utilities; benchmarked, no side effects
- `rho-tui/src/` (non-editor) — rendering components, theme, fuzzy match; visual-only impact
- `rho/src/commands/` — slash command handlers; each is isolated
- `rho/src/tui/` — app-level TUI wiring (chat display, status bar); cosmetic
- `benches/`, `tests/`, `docs/` — no runtime impact

## Change Playbooks

See **Validation Matrix** for required verification commands per change scope.

### Adding a tool

1. Create `crates/rho/src/tools/<name>.rs` implementing `rho_agent::tools::Tool` (requires `name`, `description`, `input_schema`, `execute`)
2. Choose concurrency mode: `Shared` (default, runs in parallel) or `Exclusive` (flushes pending tools, runs alone)
3. Register in `crates/rho/src/tools/mod.rs::create_default_registry()` via `builder.register(Box::new(YourTool))`
4. Add tests calling `.execute(&json!({...}), ...)` directly — no agent loop needed

### Adding an LLM provider

1. Create `crates/rho-ai/src/providers/<name>.rs` implementing `rho_ai::provider::Provider` (`name`, `stream`)
2. Add API enum variant to `rho-ai/src/models.rs::Api` and wire it in `rho-ai/src/stream.rs` provider resolution
3. Handle API key resolution, HTTP request construction, SSE response parsing, and retryable error classification
4. Add message conversion logic — map `rho-ai::types::Message` variants to the provider's wire format
5. Add tests for: request body construction, response parsing (text, tool calls, thinking), error mapping, tool result conversion

### Adding a slash command

1. Add command metadata (name, aliases, description) to `crates/rho/src/commands/registry.rs`
2. Create handler in `crates/rho/src/commands/handlers/` — receives `CommandContext`, returns `CommandResult`
3. Wire the handler in `crates/rho/src/commands/handlers/mod.rs`
4. `CommandResult` variants: `Message` (show text), `Exit`, `NewSession`, `ResumeSession`, `ModelChanged`, `Noop`

### Adding a session entry type

1. Add variant to `crates/rho/src/session/types.rs::SessionEntry` enum with `#[serde(rename_all = "camelCase")]`
2. Use a unique `type` discriminator string — existing sessions with unknown types are skipped on read (forward compatible)
3. Add serialization roundtrip test

### Changing cross-crate types

Changes to `rho-agent/src/types.rs` or `rho-ai/src/types.rs` cascade widely. Follow this order:

1. Change the type definition
2. Update `rho-agent/src/convert.rs` (agent ↔ rho-ai mapping)
3. Update all provider conversion code in `rho-ai/src/providers/*.rs`
4. Update session serialization if the type is persisted

## Validation Matrix

Required before any code commit:

```bash
cargo clippy --workspace          # must pass with zero warnings
cargo test --workspace            # all tests must pass
cargo fmt --all -- --check        # formatting must be clean
```

Additional checks by change type:

| Change scope | Required commands | Notes |
|---|---|---|
| Docs, comments, prompts | `cargo fmt --all -- --check` | Verify no broken `include_str!` paths |
| Single tool implementation | `cargo test -p rho && cargo clippy -p rho` | Tool tests are self-contained |
| `rho-ai` types or providers | `cargo test -p rho-ai -p rho-agent && cargo clippy --workspace` | Type changes cascade to agent convert layer |
| `rho-agent` core (agent_loop, tools trait, registry) | `cargo test --workspace && cargo clippy --workspace` | Cross-crate trait boundary; test everything |
| Performance-sensitive paths | Above + `cargo bench -p <crate> --bench <name>` | Compare against baseline in `target/criterion/` |
| Session format changes | `cargo test -p rho` + manual roundtrip test with existing `.rho/` session file | Verify backward compat; old sessions must still load |
| Vendored crates (`brush-*`) | `cargo test -p brush-core -p brush-builtins && cargo build` | Do not modify lint configs; test in isolation |

If full validation is impractical (e.g. CI-only tests, hardware-dependent), document what was run and what was skipped.

### Benchmarks

The project uses [criterion](https://github.com/bheisler/criterion.rs). The `[profile.bench]` in root `Cargo.toml` uses `opt-level = 3` with `lto = "thin"`. HTML reports go to `target/criterion/`.

```bash
cargo bench -p rho-text --bench text -- visible_width   # specific group
cargo bench -p rho-text --bench text -- "emoji_zwj"     # single case
```

Benchmark files:
- `crates/rho-text/benches/text.rs` — visible_width, wrap, truncate, slice, sanitize
- `crates/rho-tui/benches/keys.rs` — parse_key, matches_key, kitty_sequence
- `crates/rho-tui/benches/stdin_buffer.rs` — StdinBuffer::process
- `crates/rho-tui/benches/markdown.rs` — Markdown::render_mut

## Privacy and Sensitive Data

- Never commit real API keys, tokens, credentials, or private URLs.
- Use neutral placeholders in tests: `"test-key"`, `"example.com"`, `"user_a"`.
- Test fixtures must be impersonal — no real user data or personal information.
- Review `git diff --cached` before push for accidental sensitive strings.

## Reference

Rho is a Rust port of [oh-my-pi](https://github.com/can1357/oh-my-pi), a TypeScript-based coding agent. The original repo serves as reference for feature parity and benchmark coverage.
