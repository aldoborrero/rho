use std::path::PathBuf;

use serde::Serialize;

/// Options for building the system prompt.
pub struct BuildOptions {
	/// If set, replaces the entire default prompt.
	pub custom_prompt:        Option<String>,
	/// Text appended after the system prompt.
	pub append_system_prompt: Option<String>,
	/// Working directory for git context and context file discovery.
	pub cwd:                  PathBuf,
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
	pub name:        String,
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
	pub path:    String,
	pub content: String,
}

/// Git repository context.
#[derive(Serialize)]
pub struct GitContext {
	pub is_repo:        bool,
	pub current_branch: String,
	pub main_branch:    String,
	pub status:         String,
	pub commits:        String,
}
