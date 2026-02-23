pub mod compaction;
mod context_files;
mod environment;
mod git;
pub mod system;
pub mod tools;
pub mod types;

pub use types::BuildOptions;
use types::{PromptContext, ToolDescription};

use crate::tools::registry::ToolRegistry;

/// Build the complete system prompt.
///
/// If `options.custom_prompt` is set, it replaces the default entirely.
/// Otherwise, gathers environment, git, and project context, then renders
/// the Jinja2 template via `MiniJinja`.
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
		.map(|t| ToolDescription { name: t.name.clone(), description: t.description.clone() })
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
