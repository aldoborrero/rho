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

# Run tests (all crates)
cargo test

# Run tests for a single crate
cargo test -p rho-ai
cargo test -p rho-agent
cargo test -p rho

# Run a single test
cargo test -p rho -- test_name

# Clippy (workspace lints are strict — pedantic + nursery + deny correctness/suspicious)
cargo clippy --workspace

# Format
cargo fmt --all
```

## Architecture

### Workspace Crates

The workspace has 6 first-party crates plus 2 vendored dependencies:

- **`rho`** — Binary crate. Entry point, CLI (clap), TUI app, config resolution, tool registration, interactive mode event loop, session management, slash commands, and Anthropic model setup. This is the "application shell" that wires everything together.

- **`rho-agent`** — Agent loop engine. Runs the autonomous LLM→tool→LLM cycle. Streams responses via `rho-ai`, executes tools from a `ToolRegistry`, handles retries, and emits `AgentEvent`s over an mpsc channel. Provider-agnostic — depends on `rho-ai` for the LLM layer.

- **`rho-ai`** — LLM provider abstraction. Defines the `Provider` trait, `Model`/`ModelRegistry`, streaming (`stream`/`complete`), SSE parsing, retry logic, and type conversions. Has concrete providers for Anthropic Messages API, OpenAI Completions, and OpenAI Responses.

- **`rho-tools`** — Pure Rust tool implementations extracted from the legacy N-API codebase. Provides clipboard, file discovery (`fd`), grep, glob, HTML-to-markdown, image processing, shell/PTY execution, process management, profiling, and workmux integration. No N-API dependencies.

- **`rho-tui`** — Terminal UI framework built on crossterm. Component model (`Component` trait), editor with vim-like motions, markdown rendering with syntax highlighting (syntect), keybindings, fuzzy matching, overlays, and theme system.

- **`rho-text`** — ANSI-aware text utilities. Width measurement, slicing, wrapping, truncation, and sanitization. Both UTF-8 and UTF-16 APIs. Zero-allocation fast paths for ASCII.

- **`brush-core-vendored`** / **`brush-builtins-vendored`** — Vendored forks of the [brush shell](https://github.com/reubeno/brush) crates, patched via `[patch.crates-io]` in the root `Cargo.toml`. These are excluded from the workspace members but included via path dependencies.

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

## Conventions

- Rust edition 2024 with nightly toolchain
- Workspace-level clippy lints: pedantic + nursery enabled as warnings, correctness + suspicious as deny
- `anyhow` for application error handling, `thiserror` for library error types
- `async-trait` for async trait definitions
- `tokio` as the async runtime (multi-threaded)
- `serde`/`serde_json` for all serialization
- Vendored crates use `[patch.crates-io]` — do not modify their lint configurations

## Benchmarks

The project uses [criterion](https://github.com/bheisler/criterion.rs) for benchmarking hot paths in `rho-text` and `rho-tui`. Benchmarks mirror the original oh-mi-pi TypeScript benchmark suite for regression parity.

```bash
# Compile benchmarks (fast check)
cargo bench --no-run

# Run all benchmarks
cargo bench

# Run a specific crate's benchmarks
cargo bench -p rho-text
cargo bench -p rho-tui

# Run a specific benchmark group
cargo bench -p rho-text --bench text -- visible_width
cargo bench -p rho-tui --bench keys -- parse_key

# Run a single case
cargo bench -p rho-text --bench text -- "visible_width/emoji_zwj"
```

Benchmark files:
- `crates/rho-text/benches/text.rs` — visible_width (26 cases), wrap, truncate, slice, sanitize
- `crates/rho-tui/benches/keys.rs` — parse_key, matches_key, kitty_sequence
- `crates/rho-tui/benches/stdin_buffer.rs` — StdinBuffer::process
- `crates/rho-tui/benches/markdown.rs` — Markdown::render_mut

HTML reports are generated in `target/criterion/`. The `[profile.bench]` in root `Cargo.toml` uses `opt-level = 3` with `lto = "thin"`.

## Reference

Rho is a Rust port of [oh-my-pi](https://github.com/can1357/oh-my-pi), a TypeScript-based coding agent. The original repo serves as reference for feature parity and benchmark coverage.
