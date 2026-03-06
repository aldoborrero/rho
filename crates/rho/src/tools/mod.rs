pub mod bash;
pub mod clipboard;
pub mod edit;
pub mod find;
pub mod fuzzy_find;
pub mod grep;
pub mod html_to_markdown;
pub mod image;
pub mod process;
pub mod read;
pub mod registry;
pub mod workmux;
pub mod write;

// Re-export Tool trait and ToolOutput from rho-agent.
use registry::{ToolRegistry, ToolRegistryBuilder};
pub use rho_agent::tools::{Concurrency, OnToolUpdate, Tool, ToolOutput};

/// Returns all built-in tool implementations as a `Vec`.
///
/// Use this when you need to merge built-in tools with extension-provided
/// tools before building a [`ToolRegistry`].
#[must_use]
pub fn builtin_tools() -> Vec<Box<dyn Tool>> {
	vec![
		Box::new(bash::BashTool),
		Box::new(read::ReadTool),
		Box::new(write::WriteTool),
		Box::new(edit::EditTool),
		Box::new(grep::GrepTool),
		Box::new(find::FindTool),
		Box::new(fuzzy_find::FuzzyFindTool),
		Box::new(clipboard::ClipboardTool),
		Box::new(html_to_markdown::HtmlToMarkdownTool),
		Box::new(process::ProcessTool),
		Box::new(image::ImageTool),
		Box::new(workmux::WorkmuxTool),
	]
}

/// Creates a [`ToolRegistry`] pre-populated with all built-in tools.
#[must_use]
pub fn create_default_registry() -> ToolRegistry {
	let mut builder = ToolRegistryBuilder::new();
	for tool in builtin_tools() {
		builder.register(tool);
	}
	builder.build()
}
