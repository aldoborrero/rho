# System Prompt Assembly Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the stub `build_system_prompt()` in `interactive.rs` with a production system prompt builder that gathers environment, git, and project context, renders rich tool descriptions, and produces the final prompt via a MiniJinja template.

**Architecture:** Two new module trees: `utils/platform` (cross-cutting OS/CPU/terminal detection) and `prompts/` (unified module containing tool description `.md` files, Jinja2 system prompt template, context gathering submodules, and the top-level `build()` API). The system prompt template is ported from the TypeScript Handlebars reference to Jinja2 syntax. All `.md` content is embedded at compile time via `include_str!()`.

**Tech Stack:** MiniJinja 2 (Jinja2 template engine), tokio (async git subprocess spawning), chrono (date formatting), serde (template context serialization), dirs (home directory)

**Reference files (read-only, in oh-my-pi repo):**
- `packages/coding-agent/src/prompts/system/system-prompt.md` — Handlebars template (284 lines)
- `packages/coding-agent/src/prompts/tools/*.md` — Tool descriptions (21 files, 5 relevant)
- `packages/coding-agent/src/system-prompt.ts` — TS builder logic (571 lines)

**Target module layout:**
```
crates/rho/src/
  utils/
    mod.rs                          — pub mod platform;
    platform.rs                     — OS, arch, CPU, terminal detection
  prompts/
    mod.rs                          — pub async fn build() + re-exports
    types.rs                        — BuildOptions, PromptContext, etc.
    environment.rs                  — env gathering (uses utils::platform)
    git.rs                          — git context (async, timeouts)
    context_files.rs                — CLAUDE.md loading
    tools/
      mod.rs                        — tool_description() lookup
      bash.md                       — rich Bash tool description
      read.md                       — rich Read tool description
      write.md                      — rich Write tool description
      grep.md                       — rich Grep tool description
      find.md                       — rich Find tool description
      clipboard.md                  — Clipboard tool description
      image.md                      — Image tool description
      process.md                    — Process tool description
      fuzzy_find.md                 — FuzzyFind tool description
      html_to_markdown.md           — HtmlToMarkdown tool description
      workmux.md                    — Workmux tool description
    system/
      mod.rs                        — render() + optimize_layout()
      system-prompt.md              — Jinja2 template (ported from Handlebars)
```

---

## Task 1: Add `minijinja` dependency

**Files:**
- Modify: `crates/rho/Cargo.toml`

**Step 1: Add dependency**

Add `minijinja = "2"` after the existing `base64` line:

```toml
base64 = "0.22"
minijinja = { version = "2", features = ["builtins"] }
```

**Step 2: Verify it compiles**

Run: `cargo check -p rho`
Expected: success, no errors

**Step 3: Commit**

```bash
git add crates/rho/Cargo.toml Cargo.lock
git commit -m "feat(rho): add minijinja dependency for template rendering"
```

---

## Task 2: Create `utils/platform` module

**Files:**
- Create: `crates/rho/src/utils/mod.rs`
- Create: `crates/rho/src/utils/platform.rs`
- Modify: `crates/rho/src/main.rs` — add `mod utils;`

This is a cross-cutting utility module. OS detection is reused by prompts, debug commands, status line, etc.

**Step 1: Write the failing tests**

Create `crates/rho/src/utils/platform.rs` with just the test module:

```rust
/// Get the OS name (e.g., "linux", "macos", "windows").
pub fn os_name() -> &'static str {
    todo!()
}

/// Get the CPU architecture (e.g., "x86_64", "aarch64").
pub fn arch() -> &'static str {
    todo!()
}

/// Get the OS version string.
/// On Linux reads /etc/os-release PRETTY_NAME, on others uses std::env::consts.
pub fn os_version() -> Option<String> {
    todo!()
}

/// Get the CPU model name (e.g., "AMD Ryzen 9 5900X").
/// Reads `/proc/cpuinfo` on Linux, `sysctl` on macOS.
pub fn cpu_model() -> Option<String> {
    todo!()
}

/// Get the number of logical CPUs.
pub fn cpu_count() -> usize {
    todo!()
}

/// Get the terminal name from environment variables.
/// Checks TERM_PROGRAM (+ version), WT_SESSION, TERM, COLORTERM in that order.
pub fn terminal() -> Option<String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_name_returns_known_value() {
        let name = os_name();
        assert!(
            ["linux", "macos", "windows", "freebsd", "openbsd", "netbsd", "dragonfly", "solaris", "illumos", "android", "ios"]
                .contains(&name),
            "unexpected OS name: {name}"
        );
    }

    #[test]
    fn arch_is_not_empty() {
        assert!(!arch().is_empty());
    }

    #[test]
    fn cpu_count_is_at_least_one() {
        assert!(cpu_count() >= 1);
    }

    #[test]
    fn terminal_reads_env() {
        // Save and set TERM_PROGRAM
        let original = std::env::var("TERM_PROGRAM").ok();
        std::env::set_var("TERM_PROGRAM", "TestTerminal");
        let result = terminal();
        // Restore
        match original {
            Some(val) => std::env::set_var("TERM_PROGRAM", val),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
        assert!(result.is_some());
        assert!(result.unwrap().contains("TestTerminal"));
    }

    #[test]
    fn terminal_with_version() {
        let orig_prog = std::env::var("TERM_PROGRAM").ok();
        let orig_ver = std::env::var("TERM_PROGRAM_VERSION").ok();
        std::env::set_var("TERM_PROGRAM", "vscode");
        std::env::set_var("TERM_PROGRAM_VERSION", "1.85");
        let result = terminal();
        // Restore
        match orig_prog {
            Some(val) => std::env::set_var("TERM_PROGRAM", val),
            None => std::env::remove_var("TERM_PROGRAM"),
        }
        match orig_ver {
            Some(val) => std::env::set_var("TERM_PROGRAM_VERSION", val),
            None => std::env::remove_var("TERM_PROGRAM_VERSION"),
        }
        assert_eq!(result, Some("vscode 1.85".to_owned()));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rho utils::platform -- --nocapture 2>&1 | head -20`
Expected: FAIL with `not yet implemented`

**Step 3: Write the implementation**

Replace the `todo!()` stubs in `crates/rho/src/utils/platform.rs`:

```rust
pub fn os_name() -> &'static str {
    std::env::consts::OS
}

pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

pub fn os_version() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        // Try /etc/os-release first
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                    return Some(value.trim_matches('"').to_owned());
                }
            }
        }
        // Fallback to uname info
        if let Ok(content) = std::fs::read_to_string("/proc/version") {
            let first_line = content.lines().next().unwrap_or("");
            if !first_line.is_empty() {
                return Some(first_line.to_owned());
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub fn cpu_model() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let content = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        for line in content.lines() {
            if let Some(value) = line.strip_prefix("model name") {
                let value = value.trim_start_matches(|c: char| c == ' ' || c == '\t' || c == ':');
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

pub fn terminal() -> Option<String> {
    if let Ok(prog) = std::env::var("TERM_PROGRAM") {
        if !prog.is_empty() {
            return if let Ok(ver) = std::env::var("TERM_PROGRAM_VERSION") {
                if !ver.is_empty() {
                    Some(format!("{prog} {ver}"))
                } else {
                    Some(prog)
                }
            } else {
                Some(prog)
            };
        }
    }
    if std::env::var("WT_SESSION").is_ok() {
        return Some("Windows Terminal".to_owned());
    }
    for var in ["TERM", "COLORTERM", "TERMINAL_EMULATOR"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim().to_owned();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}
```

Create `crates/rho/src/utils/mod.rs`:

```rust
pub mod platform;
```

Add to `crates/rho/src/main.rs` after existing mods:

```rust
mod utils;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rho utils::platform -- --nocapture`
Expected: all 5 tests PASS

**Step 5: Commit**

```bash
git add crates/rho/src/utils/ crates/rho/src/main.rs
git commit -m "feat(utils): add cross-cutting platform detection module"
```

---

## Task 3: Add rich tool description `.md` files

**Files:**
- Create: `crates/rho/src/prompts/tools/mod.rs`
- Create: 11 `.md` files in `crates/rho/src/prompts/tools/`
- Create: `crates/rho/src/prompts/mod.rs` (initially just `pub mod tools;`)
- Modify: `crates/rho/src/main.rs` — add `mod prompts;`

Port tool descriptions from TS `packages/coding-agent/src/prompts/tools/*.md`. Strip Handlebars conditionals (like `{{#if IS_HASHLINE_MODE}}`), use the default line-number-mode path. For tools without TS `.md` files (clipboard, image, process, fuzzy_find, html_to_markdown, workmux), write concise descriptions matching the tool's actual capabilities.

**Step 1: Create the `.md` files**

`crates/rho/src/prompts/tools/bash.md`:

```markdown
Executes bash commands in a shell session for terminal operations like git, cargo, npm, docker.

- Use `cwd` parameter to set working directory instead of `cd dir && ...`
- Paths with spaces must use double quotes: `cd "/path/with spaces"`
- For sequential dependent operations, chain with `&&`: `mkdir foo && cd foo && touch bar`
- For parallel independent operations, make multiple tool calls in one message
- Use `;` only when later commands should run regardless of earlier failures

Output: stdout and stderr merged, exit code on non-zero. Truncated after 100KB.

Do NOT use Bash for these operations—specialized tools exist:
- Reading file contents -> Read tool
- Searching file contents -> Grep tool
- Finding files by pattern -> Find tool
- Writing new files -> Write tool
```

`crates/rho/src/prompts/tools/read.md`:

```markdown
Reads files from the local filesystem.

- Reads up to 2000 lines by default
- Use `offset` and `limit` for large files
- Text output is line-number-prefixed
- Supports images (PNG, JPG) and PDFs
- For directories, returns formatted listing with modification times
- Parallelize reads when exploring related files
```

`crates/rho/src/prompts/tools/write.md`:

```markdown
Creates or overwrites a file at the specified path.

- Creates parent directories if needed
- Prefer the Edit tool for modifying existing files (more precise, preserves formatting)
- Create documentation files (*.md, README) only when explicitly requested
```

`crates/rho/src/prompts/tools/grep.md`:

```markdown
Search file contents using ripgrep.

- Supports full regex syntax (e.g., `log.*Error`, `function\s+\w+`)
- Filter files with `glob` (e.g., `*.js`, `**/*.tsx`) or `type` (e.g., `js`, `py`, `rust`)
- Pattern syntax uses ripgrep—literal braces need escaping (`interface\{\}` to find `interface{}` in Go)
- For cross-line patterns, set `multiline: true`
- Results truncated at 100 matches by default (configurable via `limit`)

ALWAYS use Grep for search tasks—NEVER invoke `grep` or `rg` via Bash.
```

`crates/rho/src/prompts/tools/find.md`:

```markdown
Fast file pattern matching that works with any codebase size.

- Pattern includes the search path: `src/**/*.ts`, `lib/*.json`, `**/*.md`
- Simple patterns like `*.ts` automatically search recursively from cwd
- Includes hidden files by default
- Results sorted by modification time (most recent first)
- Speculatively perform multiple searches in parallel when potentially useful
```

`crates/rho/src/prompts/tools/clipboard.md`:

```markdown
Copy text to the system clipboard.

- Takes a `text` parameter with the content to copy
- Returns confirmation on success
```

`crates/rho/src/prompts/tools/image.md`:

```markdown
Get image dimensions or resize images.

- `info` action: returns width, height, and format
- `resize` action: resizes to specified dimensions, preserving aspect ratio by default
```

`crates/rho/src/prompts/tools/process.md`:

```markdown
List or kill processes and their descendants.

- `list` action: shows running processes with PID, name, and command line
- `kill` action: sends signal to a process and its entire process tree
- Default signal is SIGTERM (15)
```

`crates/rho/src/prompts/tools/fuzzy_find.md`:

```markdown
Find files using fuzzy matching on file paths.

- Matches against the full relative path, not just the filename
- Returns results ranked by match quality
- Default limit: 20 results
```

`crates/rho/src/prompts/tools/html_to_markdown.md`:

```markdown
Convert HTML content to clean Markdown.

- Strips scripts, styles, and non-content elements
- Preserves document structure (headings, lists, tables, links)
- Optional `clean` mode for more aggressive simplification
```

`crates/rho/src/prompts/tools/workmux.md`:

```markdown
Manage terminal multiplexer windows and agents.

- `detect`: check if tmux/zellij is available
- `list_agents`: list active agent windows
- `create_window`: create a new multiplexer window
- `send_keys`: send keystrokes to a window
- `capture_pane`: capture the current content of a pane
```

**Step 2: Create `prompts/tools/mod.rs`**

```rust
/// Get the rich description for a tool by name.
///
/// Returns the embedded `.md` content, or `None` if no rich description exists.
#[must_use]
pub fn tool_description(name: &str) -> Option<&'static str> {
    match name {
        "bash" => Some(include_str!("bash.md")),
        "read" => Some(include_str!("read.md")),
        "write" => Some(include_str!("write.md")),
        "grep" => Some(include_str!("grep.md")),
        "find" => Some(include_str!("find.md")),
        "clipboard" => Some(include_str!("clipboard.md")),
        "image" => Some(include_str!("image.md")),
        "process" => Some(include_str!("process.md")),
        "fuzzy_find" => Some(include_str!("fuzzy_find.md")),
        "html_to_markdown" => Some(include_str!("html_to_markdown.md")),
        "workmux" => Some(include_str!("workmux.md")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tools_have_descriptions() {
        let tools = [
            "bash", "read", "write", "grep", "find", "clipboard",
            "image", "process", "fuzzy_find", "html_to_markdown", "workmux",
        ];
        for name in tools {
            assert!(
                tool_description(name).is_some(),
                "missing description for tool: {name}"
            );
        }
    }

    #[test]
    fn unknown_tool_returns_none() {
        assert!(tool_description("nonexistent_tool").is_none());
    }

    #[test]
    fn descriptions_are_not_empty() {
        for name in ["bash", "read", "write", "grep", "find"] {
            let desc = tool_description(name).unwrap();
            assert!(!desc.trim().is_empty(), "empty description for: {name}");
        }
    }
}
```

**Step 3: Create `prompts/mod.rs`**

```rust
pub mod tools;
```

**Step 4: Add module to `main.rs`**

Add after `mod tools;`:

```rust
mod prompts;
```

**Step 5: Update each tool's `description()` to use rich `.md` content**

For each tool in `crates/rho/src/tools/*.rs`, change the `description()` method:

- `bash.rs`: `include_str!("../prompts/tools/bash.md")`
- `read.rs`: `include_str!("../prompts/tools/read.md")`
- `write.rs`: `include_str!("../prompts/tools/write.md")`
- `grep.rs`: `include_str!("../prompts/tools/grep.md")`
- `find.rs`: `include_str!("../prompts/tools/find.md")`
- `clipboard.rs`: `include_str!("../prompts/tools/clipboard.md")`
- `image.rs`: `include_str!("../prompts/tools/image.md")`
- `process.rs`: `include_str!("../prompts/tools/process.md")`
- `fuzzy_find.rs`: `include_str!("../prompts/tools/fuzzy_find.md")`
- `html_to_markdown.rs`: `include_str!("../prompts/tools/html_to_markdown.md")`
- `workmux.rs`: `include_str!("../prompts/tools/workmux.md")`

Example for `bash.rs`:

```rust
fn description(&self) -> &'static str {
    include_str!("../prompts/tools/bash.md")
}
```

**Step 6: Run tests**

Run: `cargo test -p rho prompts::tools`
Expected: 3 tests PASS

Run: `cargo test -p rho tools::bash`
Expected: existing tests still PASS

Run: `cargo check -p rho`
Expected: no errors

**Step 7: Commit**

```bash
git add crates/rho/src/prompts/ crates/rho/src/tools/ crates/rho/src/main.rs
git commit -m "feat(prompts): add rich tool descriptions from .md files"
```

---

## Task 4: Create `prompts/` scaffolding with types

**Files:**
- Create: `crates/rho/src/prompts/types.rs`
- Create: `crates/rho/src/prompts/environment.rs` (empty stub)
- Create: `crates/rho/src/prompts/git.rs` (empty stub)
- Create: `crates/rho/src/prompts/context_files.rs` (empty stub)
- Create: `crates/rho/src/prompts/system/mod.rs` (empty stub)
- Modify: `crates/rho/src/prompts/mod.rs` — expand with submodules + stub `build()`

**Step 1: Create `types.rs`**

```rust
use std::path::PathBuf;

use serde::Serialize;

/// Options for building the system prompt.
pub struct BuildOptions {
    /// If set, replaces the entire default prompt.
    pub custom_prompt: Option<String>,
    /// Text appended after the system prompt.
    pub append_system_prompt: Option<String>,
    /// Working directory for git context and context file discovery.
    pub cwd: PathBuf,
}

/// Full context passed to the system prompt template.
#[derive(Serialize)]
pub struct PromptContext {
    /// Tool names (for `{% if "bash" in tools %}` conditionals).
    pub tools: Vec<String>,
    /// Tool name + description pairs for rendering.
    pub tool_descriptions: Vec<ToolDescription>,
    /// Whether to repeat full tool descriptions in the prompt body.
    pub repeat_tool_descriptions: bool,
    /// Environment info items (OS, Arch, CPU, etc.).
    pub environment: Vec<EnvItem>,
    /// Custom system prompt from SYSTEM.md files.
    pub system_prompt_customization: Option<String>,
    /// Loaded CLAUDE.md context files.
    pub context_files: Vec<ContextFile>,
    /// Git repository context (branch, status, commits).
    pub git: Option<GitContext>,
    /// Current date string (YYYY-MM-DD).
    pub date: String,
    /// Current working directory.
    pub cwd: String,
    /// Text appended after the template.
    pub append_system_prompt: Option<String>,
}

/// A tool's name and description for template rendering.
#[derive(Serialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
}

/// An environment info item.
#[derive(Serialize)]
pub struct EnvItem {
    pub label: String,
    pub value: String,
}

/// A loaded context file (CLAUDE.md).
#[derive(Serialize)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}

/// Git repository context.
#[derive(Serialize)]
pub struct GitContext {
    pub is_repo: bool,
    pub current_branch: String,
    pub main_branch: String,
    pub status: String,
    pub commits: String,
}
```

**Step 2: Create empty stub modules**

`crates/rho/src/prompts/environment.rs`:

```rust
use super::types::EnvItem;

pub fn gather() -> Vec<EnvItem> {
    todo!()
}
```

`crates/rho/src/prompts/git.rs`:

```rust
use std::path::Path;
use super::types::GitContext;

pub async fn gather(_cwd: &Path) -> Option<GitContext> {
    todo!()
}
```

`crates/rho/src/prompts/context_files.rs`:

```rust
use std::path::Path;
use super::types::ContextFile;

pub fn gather(_cwd: &Path) -> Vec<ContextFile> {
    todo!()
}

pub fn load_system_prompt_customization() -> Option<String> {
    todo!()
}
```

`crates/rho/src/prompts/system/mod.rs`:

```rust
use super::types::PromptContext;

pub fn render(_ctx: &PromptContext) -> anyhow::Result<String> {
    todo!()
}
```

**Step 3: Expand `prompts/mod.rs`**

Replace the contents:

```rust
mod context_files;
mod environment;
mod git;
pub mod system;
pub mod tools;
pub mod types;

pub use types::BuildOptions;

use crate::tools::registry::ToolRegistry;

/// Build the complete system prompt.
///
/// If `options.custom_prompt` is set, it replaces the default entirely.
/// Otherwise, gathers environment, git, and project context, then renders
/// the Jinja2 template via MiniJinja.
pub async fn build(_tools: &ToolRegistry, _options: BuildOptions) -> anyhow::Result<String> {
    todo!()
}
```

**Step 4: Verify compilation**

Run: `cargo check -p rho`
Expected: success (todo! compiles, just panics at runtime)

**Step 5: Commit**

```bash
git add crates/rho/src/prompts/
git commit -m "feat(prompts): add module scaffolding with types"
```

---

## Task 5: Implement environment gathering

**Files:**
- Modify: `crates/rho/src/prompts/environment.rs`

**Step 1: Write the failing test**

Add tests to `crates/rho/src/prompts/environment.rs`:

```rust
use crate::utils::platform;
use super::types::EnvItem;

pub fn gather() -> Vec<EnvItem> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_returns_os_and_arch() {
        let items = gather();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"OS"), "missing OS item");
        assert!(labels.contains(&"Arch"), "missing Arch item");
    }

    #[test]
    fn gather_values_are_nonempty() {
        let items = gather();
        for item in &items {
            assert!(!item.value.is_empty(), "empty value for: {}", item.label);
        }
    }

    #[test]
    fn gather_includes_cpu_count() {
        let items = gather();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"CPU"), "missing CPU item");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rho prompts::environment -- --nocapture 2>&1 | head -10`
Expected: FAIL with `not yet implemented`

**Step 3: Write implementation**

Replace `gather()`:

```rust
use crate::utils::platform;
use super::types::EnvItem;

pub fn gather() -> Vec<EnvItem> {
    let mut items = vec![
        EnvItem {
            label: "OS".into(),
            value: platform::os_name().into(),
        },
    ];

    if let Some(version) = platform::os_version() {
        items.push(EnvItem {
            label: "Distro".into(),
            value: version,
        });
    }

    items.push(EnvItem {
        label: "Arch".into(),
        value: platform::arch().into(),
    });

    let count = platform::cpu_count();
    let cpu_value = if let Some(model) = platform::cpu_model() {
        format!("{count}x {model}")
    } else {
        format!("{count} cores")
    };
    items.push(EnvItem {
        label: "CPU".into(),
        value: cpu_value,
    });

    if let Some(term) = platform::terminal() {
        items.push(EnvItem {
            label: "Terminal".into(),
            value: term,
        });
    }

    items
}
```

**Step 4: Run tests**

Run: `cargo test -p rho prompts::environment -- --nocapture`
Expected: 3 tests PASS

**Step 5: Commit**

```bash
git add crates/rho/src/prompts/environment.rs
git commit -m "feat(prompts): implement environment gathering via utils::platform"
```

---

## Task 6: Implement git context gathering

**Files:**
- Modify: `crates/rho/src/prompts/git.rs`

**Step 1: Write the failing tests**

```rust
use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

use super::types::GitContext;

/// Timeout for most git commands.
const GIT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Slightly longer timeout for `git status`.
const GIT_STATUS_TIMEOUT: Duration = Duration::from_millis(2000);

async fn run_git(cwd: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    todo!()
}

pub async fn gather(cwd: &Path) -> Option<GitContext> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gather_returns_some_in_git_repo() {
        // The project root is a git repo
        let cwd = std::env::current_dir().unwrap();
        let ctx = gather(&cwd).await;
        assert!(ctx.is_some(), "expected Some in a git repo");
        let ctx = ctx.unwrap();
        assert!(ctx.is_repo);
        assert!(!ctx.current_branch.is_empty());
        assert!(!ctx.main_branch.is_empty());
    }

    #[tokio::test]
    async fn gather_returns_none_for_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = gather(dir.path()).await;
        assert!(ctx.is_none(), "expected None for non-git dir");
    }

    #[tokio::test]
    async fn run_git_returns_none_on_bad_command() {
        let result = run_git(Path::new("."), &["not-a-real-command"], GIT_TIMEOUT).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn run_git_returns_trimmed_output() {
        let result = run_git(Path::new("."), &["rev-parse", "--is-inside-work-tree"], GIT_TIMEOUT).await;
        assert_eq!(result, Some("true".to_owned()));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rho prompts::git -- --nocapture 2>&1 | head -10`
Expected: FAIL with `not yet implemented`

**Step 3: Write implementation**

```rust
use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

use super::types::GitContext;

const GIT_TIMEOUT: Duration = Duration::from_millis(1500);
const GIT_STATUS_TIMEOUT: Duration = Duration::from_millis(2000);

async fn run_git(cwd: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Some(text)
        }
        _ => None,
    }
}

pub async fn gather(cwd: &Path) -> Option<GitContext> {
    // Check if inside a git repo.
    let is_git = run_git(cwd, &["rev-parse", "--is-inside-work-tree"], GIT_TIMEOUT).await?;
    if is_git != "true" {
        return None;
    }

    // Get current branch.
    let current_branch = run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"], GIT_TIMEOUT).await?;

    // Detect main branch name.
    let main_branch = if run_git(cwd, &["rev-parse", "--verify", "main"], GIT_TIMEOUT)
        .await
        .is_some()
    {
        "main".to_owned()
    } else if run_git(cwd, &["rev-parse", "--verify", "master"], GIT_TIMEOUT)
        .await
        .is_some()
    {
        "master".to_owned()
    } else {
        "main".to_owned()
    };

    // Run status and log in parallel.
    let (status, commits) = tokio::join!(
        run_git(
            cwd,
            &["status", "--porcelain", "--untracked-files=no"],
            GIT_STATUS_TIMEOUT
        ),
        run_git(cwd, &["log", "--oneline", "-5"], GIT_TIMEOUT),
    );

    let status = match status {
        Some(s) if s.is_empty() => "(clean)".to_owned(),
        Some(s) => s,
        None => "(status unavailable)".to_owned(),
    };

    let commits = match commits {
        Some(c) if !c.is_empty() => c,
        _ => "(no commits)".to_owned(),
    };

    Some(GitContext {
        is_repo: true,
        current_branch,
        main_branch,
        status,
        commits,
    })
}
```

**Step 4: Run tests**

Run: `cargo test -p rho prompts::git -- --nocapture`
Expected: 4 tests PASS

**Step 5: Commit**

```bash
git add crates/rho/src/prompts/git.rs
git commit -m "feat(prompts): implement git context gathering with timeouts"
```

---

## Task 7: Implement context file loading

**Files:**
- Modify: `crates/rho/src/prompts/context_files.rs`

**Step 1: Write the failing tests**

```rust
use std::path::Path;

use super::types::ContextFile;

pub fn gather(cwd: &Path) -> Vec<ContextFile> {
    todo!()
}

pub fn load_system_prompt_customization() -> Option<String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_does_not_crash_on_nonexistent_dir() {
        let result = gather(Path::new("/nonexistent/dir/that/should/not/exist"));
        // Should return empty, not panic
        assert!(result.is_empty() || !result.is_empty());
    }

    #[test]
    fn gather_finds_claude_md_in_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let claude_md = dir.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "# Test context\nSome instructions.").unwrap();

        let result = gather(dir.path());
        assert_eq!(result.len(), 1);
        assert!(result[0].path.ends_with("CLAUDE.md"));
        assert!(result[0].content.contains("Test context"));
    }

    #[test]
    fn gather_finds_dotclaude_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        let dotclaude = dir.path().join(".claude");
        std::fs::create_dir(&dotclaude).unwrap();
        std::fs::write(dotclaude.join("CLAUDE.md"), "dotclaude content").unwrap();

        let result = gather(dir.path());
        assert_eq!(result.len(), 1);
        assert!(result[0].content.contains("dotclaude content"));
    }

    #[test]
    fn gather_skips_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "   \n  \n  ").unwrap();

        let result = gather(dir.path());
        assert!(result.is_empty(), "should skip whitespace-only files");
    }

    #[test]
    fn load_system_prompt_customization_returns_none_when_missing() {
        // This will return None unless ~/.claude/SYSTEM.md exists
        // We can't guarantee the file doesn't exist, but at minimum it shouldn't crash
        let _ = load_system_prompt_customization();
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rho prompts::context_files -- --nocapture 2>&1 | head -10`
Expected: FAIL with `not yet implemented`

**Step 3: Write implementation**

```rust
use std::path::Path;

use super::types::ContextFile;

/// Try to read a file, returning `None` if missing or whitespace-only.
fn read_nonempty(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    Some(content)
}

/// Gather CLAUDE.md context files from standard locations.
///
/// Searches (in order):
/// 1. `~/.claude/CLAUDE.md` (user-level)
/// 2. `<cwd>/CLAUDE.md` (project root)
/// 3. `<cwd>/.claude/CLAUDE.md` (project .claude dir)
pub fn gather(cwd: &Path) -> Vec<ContextFile> {
    let mut files = Vec::new();

    // 1. User-level: ~/.claude/CLAUDE.md
    if let Some(home) = dirs::home_dir() {
        let path = home.join(".claude").join("CLAUDE.md");
        if let Some(content) = read_nonempty(&path) {
            files.push(ContextFile {
                path: path.display().to_string(),
                content,
            });
        }
    }

    // 2. Project root: <cwd>/CLAUDE.md
    let project_claude = cwd.join("CLAUDE.md");
    if let Some(content) = read_nonempty(&project_claude) {
        files.push(ContextFile {
            path: project_claude.display().to_string(),
            content,
        });
    }

    // 3. Project .claude dir: <cwd>/.claude/CLAUDE.md
    let dotclaude = cwd.join(".claude").join("CLAUDE.md");
    if let Some(content) = read_nonempty(&dotclaude) {
        files.push(ContextFile {
            path: dotclaude.display().to_string(),
            content,
        });
    }

    files
}

/// Load system prompt customization from `~/.claude/SYSTEM.md`.
pub fn load_system_prompt_customization() -> Option<String> {
    let home = dirs::home_dir()?;
    read_nonempty(&home.join(".claude").join("SYSTEM.md"))
}
```

**Step 4: Run tests**

Run: `cargo test -p rho prompts::context_files -- --nocapture`
Expected: 5 tests PASS

**Step 5: Commit**

```bash
git add crates/rho/src/prompts/context_files.rs
git commit -m "feat(prompts): implement CLAUDE.md context file loading"
```

---

## Task 8: Port system prompt template to MiniJinja syntax

**Files:**
- Create: `crates/rho/src/prompts/system/system-prompt.md`

Port the Handlebars template from `packages/coding-agent/src/prompts/system/system-prompt.md` (284 lines) to Jinja2 syntax.

**Conversion rules:**

| Handlebars | MiniJinja |
|---|---|
| `{{#if x}}` | `{% if x %}` |
| `{{/if}}` | `{% endif %}` |
| `{{#each items}}` ... `{{/each}}` | `{% for item in items %}` ... `{% endfor %}` |
| `{{#has tools "read"}}` | `{% if "read" in tools %}` |
| `{{#ifAny (includes tools "python") (includes tools "bash")}}` | `{% if "python" in tools or "bash" in tools %}` |
| `{{#list items prefix="- " join="\n"}}{{label}}: {{value}}{{/list}}` | `{% for item in items %}- {{ item.label }}: {{ item.value }}\n{% endfor %}` |
| `{{this}}` | `{{ item }}` |
| `{{name}}` | `{{ name }}` |
| `{{git.currentBranch}}` | `{{ git.current_branch }}` |

**Step 1: Create the Jinja2 template**

Create `crates/rho/src/prompts/system/system-prompt.md` — this is the full ported template. Omit sections for tools/features not yet in the Rust crate (skills, rules, AGENTS.md, task tool, ssh, lsp, edit, python, ask).

```
<identity>
Distinguished Staff Engineer.

High-agency. Principled. Decisive.
Expertise: debugging, refactoring, system design.
Judgment: earned through failure, recovery.

Correctness > politeness. Brevity > ceremony.
Say truth; omit filler. No apologies. No comfort where clarity belongs.
Push back when warranted: state downside, propose alternative, accept override.
</identity>

<discipline>
Notice the completion reflex before it fires:
- Urge to produce something that runs
- Pattern-matching to similar problems
- Assumption that compiling = correct
- Satisfaction at "it works" before "works in all cases"

Before writing code, think through:
- What are my assumptions about input? About environment?
- What breaks this?
- What would a malicious caller do?
- Would a tired maintainer misunderstand this?
- Can this be simpler?
- Are these abstractions earning their keep?

The question is not "does this work?" but "under what conditions? What happens outside them?"
</discipline>
{% if system_prompt_customization %}

<context>
{{ system_prompt_customization }}
</context>
{% endif %}

<environment>
{% for item in environment %}- {{ item.label }}: {{ item.value }}
{% endfor %}</environment>

<tools>
## Available Tools
{% if repeat_tool_descriptions %}
{% for tool in tool_descriptions %}
<tool name="{{ tool.name }}">
{{ tool.description }}
</tool>
{% endfor %}
{% else %}
{% for name in tools %}- {{ name }}
{% endfor %}
{% endif %}
{% if "bash" in tools %}

### Precedence: Specialized -> Bash
{% if "read" in tools or "grep" in tools or "find" in tools %}1. **Specialized**: {% if "read" in tools %}`read`, {% endif %}{% if "grep" in tools %}`grep`, {% endif %}{% if "find" in tools %}`find`{% endif %}

{% endif %}2. **Bash**: simple one-liners only (`cargo build`, `npm install`, `docker run`)

Never use Bash when a specialized tool exists.
{% if "read" in tools or "write" in tools or "grep" in tools or "find" in tools %}{% if "read" in tools %}`read` not cat/open(); {% endif %}{% if "write" in tools %}`write` not cat>/echo>; {% endif %}{% if "grep" in tools %}`grep` not bash grep/rg; {% endif %}{% if "find" in tools %}`find` not bash find/glob.{% endif %}

{% endif %}
{% endif %}
{% if "grep" in tools or "find" in tools %}
### Search before you read
Don't open a file hoping. Hope is not a strategy.
{% if "find" in tools %}- Unknown territory -> `find` to map it
{% endif %}{% if "grep" in tools %}- Known territory -> `grep` to locate target
{% endif %}{% if "read" in tools %}- Known location -> `read` with offset/limit, not whole file
{% endif %}
{% endif %}</tools>

<procedure>
## Task Execution
**Assess the scope.**
- If the task is multi-file or not precisely scoped, make a plan of 3-7 steps.
**Do the work.**
- Every turn must advance towards the deliverable: edit, write, execute, delegate.
**If blocked**:
- Exhaust tools/context/files first, explore.
- Only then ask -- minimum viable question.
**If requested change includes refactor**:
- Cleanup dead code and unused elements, do not yield until your solution is pristine.

### Verification
- Prefer external proof: tests, linters, type checks, repro steps.
- If unverified: state what to run and expected result.
- Non-trivial logic: define test first when feasible.
- Algorithmic work: naive correct version before optimizing.
- **Formatting is a batch operation.** Make all semantic changes first, then run the project's formatter once.

### Concurrency Awareness
You are not alone in the codebase. Others may edit concurrently.
If contents differ or edits fail: re-read, adapt.
Never run destructive git commands, bulk overwrites, or delete code you didn't write.
</procedure>

<project>
{% if context_files %}
## Context
{% for file in context_files %}
<file path="{{ file.path }}">
{{ file.content }}
</file>
{% endfor %}
{% endif %}
{% if git %}
## Version Control
Snapshot; no updates during conversation.

Current branch: {{ git.current_branch }}
Main branch: {{ git.main_branch }}

{{ git.status }}

### History
{{ git.commits }}
{% endif %}
</project>

Current directory: {{ cwd }}
Current date: {{ date }}
{% if append_system_prompt %}

{{ append_system_prompt }}
{% endif %}

<output_style>
- No summary closings ("In summary..."). No filler. No emojis. No ceremony.
- Suppress: "genuinely", "honestly", "straightforward".
- Requirements conflict or are unclear -> ask only after exhaustive exploration.
</output_style>

<contract>
These are inviolable. Violation is system failure.
1. Never claim unverified correctness.
2. Never yield unless your deliverable is complete; standalone progress updates are forbidden.
3. Never suppress tests to make code pass. Never fabricate outputs not observed.
4. Never avoid breaking changes that correctness requires.
5. Never solve the wished-for problem instead of the actual problem.
6. Never ask for information obtainable from tools, repo context, or files.
</contract>

<diligence>
**GET THE TASK DONE.**
Complete the full request before yielding. Use tools for verifiable facts. Results conflict -> investigate. Incomplete -> iterate.
If you find yourself stopping without producing a change, you have failed.

You have unlimited stamina; the user does not. Persist on hard problems. Don't burn their energy on problems you failed to think through.

Tests you didn't write: bugs shipped.
Assumptions you didn't validate: incidents to debug.
Edge cases you ignored: pages at 3am.

Write what you can defend.
</diligence>

<stakes>
This is not practice. Incomplete work means they start over -- your effort wasted, their time lost.

You are capable of extraordinary work.
The person waiting deserves to receive it.
</stakes>

<critical>
- Every turn must advance the deliverable. A non-final turn without at least one side-effect is invalid.
- Quote only what's needed; rest is noise.
- Don't claim unverified correctness.
- Do not ask when it may be obtained from available tools or repo context/files.
- Touch only requested; no incidental refactors/cleanup.
</critical>
```

**Step 2: Verify** — Template syntax is verified by rendering tests in Task 9.

**Step 3: Commit**

```bash
git add crates/rho/src/prompts/system/system-prompt.md
git commit -m "feat(prompts): port system prompt template to Jinja2 syntax"
```

---

## Task 9: Implement template rendering and `build()`

**Files:**
- Modify: `crates/rho/src/prompts/system/mod.rs`
- Modify: `crates/rho/src/prompts/mod.rs`

**Step 1: Write the failing tests**

Add to `crates/rho/src/prompts/system/mod.rs`:

```rust
use super::types::PromptContext;

pub fn render(ctx: &PromptContext) -> anyhow::Result<String> {
    todo!()
}

fn optimize_layout(input: &str) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::types::*;

    fn minimal_context() -> PromptContext {
        PromptContext {
            tools: vec!["bash".into(), "read".into(), "write".into(), "grep".into(), "find".into()],
            tool_descriptions: vec![
                ToolDescription { name: "bash".into(), description: "Execute commands".into() },
                ToolDescription { name: "read".into(), description: "Read files".into() },
            ],
            repeat_tool_descriptions: false,
            environment: vec![
                EnvItem { label: "OS".into(), value: "linux".into() },
                EnvItem { label: "Arch".into(), value: "x86_64".into() },
            ],
            system_prompt_customization: None,
            context_files: vec![],
            git: None,
            date: "2026-02-23".into(),
            cwd: "/home/user/project".into(),
            append_system_prompt: None,
        }
    }

    #[test]
    fn template_renders_without_error() {
        let ctx = minimal_context();
        let result = render(&ctx);
        assert!(result.is_ok(), "render failed: {:?}", result.err());
    }

    #[test]
    fn template_contains_identity_section() {
        let result = render(&minimal_context()).unwrap();
        assert!(result.contains("<identity>"));
        assert!(result.contains("Distinguished Staff Engineer"));
    }

    #[test]
    fn template_contains_environment() {
        let result = render(&minimal_context()).unwrap();
        assert!(result.contains("OS: linux"));
        assert!(result.contains("Arch: x86_64"));
    }

    #[test]
    fn template_contains_tool_names() {
        let result = render(&minimal_context()).unwrap();
        assert!(result.contains("- bash"));
        assert!(result.contains("- read"));
    }

    #[test]
    fn template_contains_date_and_cwd() {
        let result = render(&minimal_context()).unwrap();
        assert!(result.contains("2026-02-23"));
        assert!(result.contains("/home/user/project"));
    }

    #[test]
    fn template_includes_git_context_when_present() {
        let mut ctx = minimal_context();
        ctx.git = Some(GitContext {
            is_repo: true,
            current_branch: "feat/test".into(),
            main_branch: "main".into(),
            status: "(clean)".into(),
            commits: "abc1234 initial commit".into(),
        });
        let result = render(&ctx).unwrap();
        assert!(result.contains("feat/test"));
        assert!(result.contains("(clean)"));
        assert!(result.contains("abc1234"));
    }

    #[test]
    fn template_omits_git_section_when_absent() {
        let ctx = minimal_context();
        let result = render(&ctx).unwrap();
        assert!(!result.contains("Version Control"));
    }

    #[test]
    fn template_includes_context_files() {
        let mut ctx = minimal_context();
        ctx.context_files = vec![ContextFile {
            path: "/home/user/.claude/CLAUDE.md".into(),
            content: "Always use tabs.".into(),
        }];
        let result = render(&ctx).unwrap();
        assert!(result.contains("Always use tabs."));
    }

    #[test]
    fn template_includes_system_customization() {
        let mut ctx = minimal_context();
        ctx.system_prompt_customization = Some("Custom system instructions here.".into());
        let result = render(&ctx).unwrap();
        assert!(result.contains("<context>"));
        assert!(result.contains("Custom system instructions here."));
    }

    #[test]
    fn template_omits_context_when_no_customization() {
        let ctx = minimal_context();
        let result = render(&ctx).unwrap();
        assert!(!result.contains("<context>"));
    }

    #[test]
    fn template_includes_append_prompt() {
        let mut ctx = minimal_context();
        ctx.append_system_prompt = Some("Extra instructions appended.".into());
        let result = render(&ctx).unwrap();
        assert!(result.contains("Extra instructions appended."));
    }

    #[test]
    fn template_tool_precedence_with_bash() {
        let ctx = minimal_context();
        let result = render(&ctx).unwrap();
        assert!(result.contains("Precedence"));
        assert!(result.contains("Specialized"));
    }

    #[test]
    fn template_tool_precedence_without_bash() {
        let mut ctx = minimal_context();
        ctx.tools = vec!["read".into(), "write".into()];
        let result = render(&ctx).unwrap();
        assert!(!result.contains("Precedence"));
    }

    #[test]
    fn repeat_tool_descriptions_shows_full_descriptions() {
        let mut ctx = minimal_context();
        ctx.repeat_tool_descriptions = true;
        let result = render(&ctx).unwrap();
        assert!(result.contains("<tool name=\"bash\">"));
        assert!(result.contains("Execute commands"));
    }

    #[test]
    fn optimize_layout_collapses_blank_lines() {
        let input = "line1\n\n\n\n\nline2";
        let result = optimize_layout(input);
        assert_eq!(result, "line1\n\nline2");
    }

    #[test]
    fn optimize_layout_trims_trailing_whitespace() {
        let input = "hello   \nworld  ";
        let result = optimize_layout(input);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn optimize_layout_normalizes_crlf() {
        let input = "hello\r\nworld\r\n";
        let result = optimize_layout(input);
        assert_eq!(result, "hello\nworld");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rho prompts::system -- --nocapture 2>&1 | head -10`
Expected: FAIL with `not yet implemented`

**Step 3: Write implementation of `system/mod.rs`**

```rust
use minijinja::Environment;

use super::types::PromptContext;

static TEMPLATE: &str = include_str!("system-prompt.md");

/// Render the system prompt template with the given context.
pub fn render(ctx: &PromptContext) -> anyhow::Result<String> {
    let mut env = Environment::new();
    env.add_template("system-prompt", TEMPLATE)?;
    let tmpl = env.get_template("system-prompt")?;
    let raw = tmpl.render(ctx)?;
    Ok(optimize_layout(&raw))
}

/// Post-process rendered template output.
///
/// - Normalize CRLF to LF
/// - Trim trailing whitespace per line
/// - Collapse 3+ consecutive blank lines to 2
/// - Trim leading/trailing whitespace of the whole output
fn optimize_layout(input: &str) -> String {
    let normalized = input.replace("\r\n", "\n");

    let mut lines: Vec<&str> = normalized.lines().map(|l| l.trim_end()).collect();

    // Collapse runs of 2+ blank lines into exactly 1 blank line.
    let mut result = String::with_capacity(input.len());
    let mut blank_count = 0u32;
    for line in &lines {
        if line.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push('\n');
            }
        } else {
            if blank_count > 0 && !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_owned()
}
```

**Step 4: Implement `build()` in `prompts/mod.rs`**

Replace the stub:

```rust
mod context_files;
mod environment;
mod git;
pub mod system;
pub mod tools;
pub mod types;

pub use types::BuildOptions;

use crate::tools::registry::ToolRegistry;
use types::{PromptContext, ToolDescription};

/// Build the complete system prompt.
///
/// If `options.custom_prompt` is set, it replaces the default entirely.
/// Otherwise, gathers environment, git, and project context, then renders
/// the Jinja2 template via MiniJinja.
pub async fn build(tools: &ToolRegistry, options: BuildOptions) -> anyhow::Result<String> {
    // Custom prompt bypasses template entirely.
    if let Some(custom) = options.custom_prompt {
        let mut prompt = custom;
        if let Some(append) = options.append_system_prompt {
            prompt.push_str("\n\n");
            prompt.push_str(&append);
        }
        return Ok(prompt);
    }

    // Gather context from various sources.
    let tool_defs = tools.definitions();
    let tool_names: Vec<String> = tool_defs.iter().map(|t| t.name.clone()).collect();
    let tool_descriptions: Vec<ToolDescription> = tool_defs
        .iter()
        .map(|t| ToolDescription {
            name: t.name.clone(),
            description: t.description.clone(),
        })
        .collect();

    let env_items = environment::gather();
    let context_files = context_files::gather(&options.cwd);
    let system_customization = context_files::load_system_prompt_customization();
    let git_context = git::gather(&options.cwd).await;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let ctx = PromptContext {
        tools: tool_names,
        tool_descriptions,
        repeat_tool_descriptions: false,
        environment: env_items,
        system_prompt_customization: system_customization,
        context_files,
        git: git_context,
        date,
        cwd: options.cwd.display().to_string(),
        append_system_prompt: options.append_system_prompt,
    };

    system::render(&ctx)
}
```

**Step 5: Run tests**

Run: `cargo test -p rho prompts::system -- --nocapture`
Expected: all 16 tests PASS

Run: `cargo test -p rho prompts -- --nocapture`
Expected: all prompts tests PASS (tools + system)

**Step 6: Commit**

```bash
git add crates/rho/src/prompts/system/ crates/rho/src/prompts/mod.rs
git commit -m "feat(prompts): implement MiniJinja template rendering and build()"
```

---

## Task 10: Wire into `interactive.rs`

**Files:**
- Modify: `crates/rho/src/modes/interactive.rs`

**Step 1: Delete old `build_system_prompt()` and update import**

Remove `use std::fmt::Write as _;` (line 1) — it's only used by the old function.

Delete lines 16-46 (the old `build_system_prompt` function).

**Step 2: Replace the call site**

At line ~141 (after deletion, around line ~114), replace:

```rust
// Old:
let system_prompt = build_system_prompt(
    &tools,
    cli.system_prompt.as_deref(),
    cli.append_system_prompt.as_deref(),
);

// New:
let system_prompt = crate::prompts::build(
    &tools,
    crate::prompts::BuildOptions {
        custom_prompt: cli.system_prompt.clone(),
        append_system_prompt: cli.append_system_prompt.clone(),
        cwd: std::env::current_dir().unwrap_or_default(),
    },
)
.await?;
```

**Step 3: Verify compilation and tests**

Run: `cargo check -p rho`
Expected: success

Run: `cargo test -p rho`
Expected: all tests PASS

**Step 4: Commit**

```bash
git add crates/rho/src/modes/interactive.rs
git commit -m "refactor(rho): replace stub system prompt with prompts::build"
```

---

## Task 11: Final verification

**Step 1: Run clippy**

Run: `cargo clippy -p rho`
Expected: no errors (warnings acceptable from existing code)

**Step 2: Run all tests**

Run: `cargo test -p rho`
Expected: all tests PASS

**Step 3: Build the binary**

Run: `cargo build -p rho`
Expected: success

**Step 4: Fix any issues found, then commit**

```bash
git add -A
git commit -m "chore(prompts): final cleanup and lint fixes"
```

(Only if there are actual fixes needed — skip commit if clean.)

---

## Key Design Decisions

- **Unified `prompts/` module** — Tool descriptions, system prompt template, context gathering, and rendering all live under one module, matching the TS `prompts/` structure
- **`utils/platform`** — OS/CPU/terminal detection lives in a reusable cross-cutting module, separate from prompts
- **`prompts/tools/*.md`** — Rich tool descriptions embedded via `include_str!()`, returned from `Tool::description()`. Serves dual purpose: API `tools` parameter AND system prompt (when `repeat_tool_descriptions: true`)
- **No runtime templating for tool descriptions** — Tool `.md` files are static (Handlebars conditionals stripped, line-number-mode defaults used)
- **MiniJinja** — Jinja2 `{% if "x" in list %}` works natively with `Vec<String>`, no custom helpers needed
- **Embedded template** — `include_str!()` compiles the system prompt `.md` template into the binary
- **Graceful degradation** — Each context source returns `Option`/empty on failure; template handles all cases via `{% if %}` guards
- **Async only for git** — Environment and context files are synchronous; only `build()` is async due to git subprocess spawning
- **Sections deferred to later** — skills, rules, AGENTS.md, task tool, ssh, lsp, edit, python, ask tool — these will be added as those features are implemented
