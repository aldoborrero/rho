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
use registry::{ToolRegistry, ToolRegistryBuilder};
pub use rho_agent::tools::{Concurrency, Tool, ToolOutput};

/// Creates a [`ToolRegistry`] pre-populated with all built-in tools.
#[must_use]
pub fn create_default_registry() -> ToolRegistry {
	let mut builder = ToolRegistryBuilder::new();
	builder.register(Box::new(bash::BashTool));
	builder.register(Box::new(read::ReadTool));
	builder.register(Box::new(write::WriteTool));
	builder.register(Box::new(grep::GrepTool));
	builder.register(Box::new(find::FindTool));
	builder.register(Box::new(fuzzy_find::FuzzyFindTool));
	builder.register(Box::new(clipboard::ClipboardTool));
	builder.register(Box::new(html_to_markdown::HtmlToMarkdownTool));
	builder.register(Box::new(process::ProcessTool));
	builder.register(Box::new(image::ImageTool));
	builder.register(Box::new(workmux::WorkmuxTool));
	builder.build()
}
