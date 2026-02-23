# rho

Terminal-based AI coding agent written in Rust. Rust port of [oh-my-pi](https://github.com/can1357/oh-my-pi).

> Early stage -- expect rough edges.

## Quick start

Requires Nix with flakes enabled:

```bash
direnv allow   # or: nix develop
export ANTHROPIC_API_KEY=sk-...
cargo run -p rho
```

Or with a direct prompt:

```bash
cargo run -p rho -- "fix the failing test in src/lib.rs"
```

## Usage

```
rho [OPTIONS] [MESSAGE]...

Options:
  -m, --model <MODEL>        Model name [default: claude-sonnet-4-5-20250929]
  -c, --continue              Continue most recent session
  -r, --resume <ID>          Resume specific session by ID
  -p, --print                Non-interactive print mode
      --thinking <LEVEL>     Thinking level [default: off]
      --no-session           Ephemeral session (no persistence)
      --api-key <KEY>        Anthropic API key (overrides env/config)
      --system-prompt <TEXT>  Override system prompt
```

## Building

```bash
cargo build            # debug
cargo build --release  # release (LTO + strip)
cargo test             # all crates
cargo clippy --workspace
cargo bench            # criterion benchmarks
```

## Architecture

Six workspace crates:

- **rho** -- binary, CLI, TUI, config, session management
- **rho-agent** -- LLM agent loop (stream -> tool -> stream cycle)
- **rho-ai** -- provider abstraction (Anthropic, OpenAI)
- **rho-tools** -- tool implementations (bash, file ops, grep, etc.)
- **rho-tui** -- terminal UI framework (crossterm, markdown, keybindings)
- **rho-text** -- ANSI-aware text utilities (width, wrap, slice, truncate)

Plus two vendored [brush](https://github.com/reubeno/brush) shell crates.

## License

MIT
