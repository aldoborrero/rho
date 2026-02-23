pub mod bash;
pub mod clipboard;
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
use registry::ToolRegistry;
pub use rho_agent::tools::{Tool, ToolOutput};

/// Creates a [`ToolRegistry`] pre-populated with all built-in tools.
#[must_use]
pub fn create_default_registry() -> ToolRegistry {
	let mut registry = ToolRegistry::new();
	registry.register(Box::new(bash::BashTool));
	registry.register(Box::new(read::ReadTool));
	registry.register(Box::new(write::WriteTool));
	registry.register(Box::new(grep::GrepTool));
	registry.register(Box::new(find::FindTool));
	registry.register(Box::new(fuzzy_find::FuzzyFindTool));
	registry.register(Box::new(clipboard::ClipboardTool));
	registry.register(Box::new(html_to_markdown::HtmlToMarkdownTool));
	registry.register(Box::new(process::ProcessTool));
	registry.register(Box::new(image::ImageTool));
	registry.register(Box::new(workmux::WorkmuxTool));
	registry
}
