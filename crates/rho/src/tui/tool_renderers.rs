//! Per-tool rendering functions that produce styled ANSI output.
//!
//! Each renderer maps a tool call or tool result into `Vec<String>` of styled
//! lines, using `OutputBlock` for bordered display and inline text for
//! lightweight tools like Read.

use rho_tui::{
	components::output_block::{
		OutputBlockOptions, OutputBlockState, OutputSection, render_output_block,
	},
	highlight::{highlight_code, language_from_path},
	symbols::TreeSymbols,
	theme::{Theme, ThemeBg, ThemeColor},
};
use serde_json::Value;

// ── Types ───────────────────────────────────────────────────────────────

/// Display info for a tool result.
pub struct ToolResultDisplay {
	pub content:  String,
	pub is_error: bool,
}

/// Trait for per-tool rendering.
pub trait ToolRenderer {
	/// Render the tool call phase (before result).
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String>;
	/// Render the tool result phase.
	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String>;
	/// Render a combined block merging call info + result into one bordered
	/// block. Default delegates to `render_result`.
	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let _ = args; // unused in default impl
		self.render_result(result, expanded, theme, width)
	}
}

// ── Style helpers ───────────────────────────────────────────────────────

pub(crate) fn make_border_style(
	theme: &Theme,
	state: OutputBlockState,
) -> Box<dyn Fn(&str) -> String> {
	let color = match state {
		OutputBlockState::Pending | OutputBlockState::Running => ThemeColor::Accent,
		OutputBlockState::Success => ThemeColor::Dim,
		OutputBlockState::Error | OutputBlockState::Warning => ThemeColor::Error,
	};
	theme.border_color_fn(color)
}

#[allow(clippy::type_complexity, reason = "matches OutputBlockOptions::bg_style signature")]
pub(crate) fn make_bg_style(
	theme: &Theme,
	state: OutputBlockState,
) -> Option<Box<dyn Fn(&str) -> String>> {
	let bg = match state {
		OutputBlockState::Pending | OutputBlockState::Running => ThemeBg::ToolPendingBg,
		OutputBlockState::Success => ThemeBg::ToolSuccessBg,
		OutputBlockState::Error | OutputBlockState::Warning => ThemeBg::ToolErrorBg,
	};
	let ansi = theme.bg_ansi(bg).to_owned();
	if ansi.is_empty() {
		None
	} else {
		Some(Box::new(move |s: &str| format!("{ansi}{s}\x1b[49m")))
	}
}

/// Collapse lines to at most `max` visible lines, appending a dim
/// `"... (N more lines)"` indicator when truncated.
pub(crate) fn collapse_lines(lines: &[&str], max: usize, theme: &Theme) -> Vec<String> {
	if lines.len() <= max {
		return lines.iter().map(|s| (*s).to_owned()).collect();
	}
	let mut out: Vec<String> = lines[..max].iter().map(|s| (*s).to_owned()).collect();
	let more = lines.len() - max;
	out.push(theme.dim(&format!("\u{2026} ({more} more lines)")));
	out
}

/// Extract a string field from JSON args, returning a default if absent.
fn arg_str<'a>(args: &'a Value, field: &str) -> &'a str {
	args.get(field).and_then(Value::as_str).unwrap_or("")
}

/// Extract the file path from Read tool args (checks both `"path"` and
/// `"file_path"` for compatibility).
fn read_path(args: &Value) -> &str {
	args
		.get("path")
		.or_else(|| args.get("file_path"))
		.and_then(Value::as_str)
		.unwrap_or("")
}

/// Common setup for tool result blocks: state, icon, header, content
/// processing.
struct ToolBlockSetup {
	header_text:  String,
	header_width: usize,
	state:        OutputBlockState,
	collapsed:    Vec<String>,
}

/// Build the shared preamble used by both [`render_result_block`] and
/// [`render_combined_block`].
fn build_tool_block_setup(
	tool_name: &str,
	result: &ToolResultDisplay,
	collapsed_lines: usize,
	expanded_lines: usize,
	expanded: bool,
	theme: &Theme,
) -> ToolBlockSetup {
	let state = if result.is_error {
		OutputBlockState::Error
	} else {
		OutputBlockState::Success
	};
	let icon = if result.is_error {
		"\u{2718}"
	} else {
		"\u{2714}"
	};
	let header_text = theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("{icon} {tool_name}")));
	let header_width = rho_text::width::visible_width_str(&header_text);
	let max_lines = if expanded {
		expanded_lines
	} else {
		collapsed_lines
	};
	let content_lines: Vec<&str> = result.content.lines().collect();
	let collapsed = collapse_lines(&content_lines, max_lines, theme);
	ToolBlockSetup { header_text, header_width, state, collapsed }
}

/// Render a standard bordered result block (shared by all non-inline
/// renderers).
#[allow(
	clippy::too_many_arguments,
	reason = "private helper consolidating 5 near-identical render_result bodies"
)]
fn render_result_block(
	tool_name: &str,
	result: &ToolResultDisplay,
	section_label: Option<&str>,
	collapsed_lines: usize,
	expanded_lines: usize,
	expanded: bool,
	theme: &Theme,
	width: u16,
) -> Vec<String> {
	let setup =
		build_tool_block_setup(tool_name, result, collapsed_lines, expanded_lines, expanded, theme);
	let opts = OutputBlockOptions {
		header:       setup.header_text,
		header_width: setup.header_width,
		state:        setup.state,
		sections:     vec![OutputSection {
			label: section_label.map(|l| theme.dim(l)),
			lines: setup.collapsed,
		}],
		border_style: make_border_style(theme, setup.state),
		bg_style:     make_bg_style(theme, setup.state),
	};
	render_output_block(&opts, width)
}

/// Render a combined call + result block. Takes pre-built sections for the call
/// info, then appends a result section with collapsed content.
#[allow(clippy::too_many_arguments, reason = "private helper consolidating combined render logic")]
fn render_combined_block(
	tool_name: &str,
	call_sections: Vec<OutputSection>,
	result: &ToolResultDisplay,
	result_label: Option<&str>,
	collapsed_lines: usize,
	expanded_lines: usize,
	expanded: bool,
	theme: &Theme,
	width: u16,
) -> Vec<String> {
	let setup =
		build_tool_block_setup(tool_name, result, collapsed_lines, expanded_lines, expanded, theme);
	let mut sections = call_sections;
	sections
		.push(OutputSection { label: result_label.map(|l| theme.dim(l)), lines: setup.collapsed });
	let opts = OutputBlockOptions {
		header: setup.header_text,
		header_width: setup.header_width,
		state: setup.state,
		sections,
		border_style: make_border_style(theme, setup.state),
		bg_style: make_bg_style(theme, setup.state),
	};
	render_output_block(&opts, width)
}

// ── Code Cell ───────────────────────────────────────────────────────────

/// Options for rendering a syntax-highlighted code cell.
pub struct CodeCellOptions<'a> {
	/// Source code to display.
	pub code:           &'a str,
	/// Language identifier for syntax highlighting.
	pub language:       Option<&'a str>,
	/// Pre-styled header string (e.g. icon + tool name + file path).
	pub title:          String,
	/// Visual state of the block.
	pub state:          OutputBlockState,
	/// Additional output lines (warnings, truncation info) shown below code.
	pub output_lines:   Vec<String>,
	/// Whether the block is expanded to show all lines.
	pub expanded:       bool,
	/// Maximum visible code lines when collapsed (default: 12).
	pub code_max_lines: usize,
}

/// Render a bordered code cell with syntax highlighting.
///
/// Produces a bordered output block containing highlighted code, optionally
/// collapsed to `code_max_lines`, with an optional "Output" section for
/// warnings or metadata.
pub fn render_code_cell(opts: &CodeCellOptions, theme: &Theme, width: u16) -> Vec<String> {
	let colors = theme.highlight_colors();
	let highlighted = highlight_code(opts.code, opts.language, &colors);

	// Expand tabs to spaces so visible_width_str matches terminal rendering.
	// highlight_code preserves literal tabs, but terminals render them with
	// variable width (position-dependent). Replacing with TAB_WIDTH spaces
	// ensures the padding calculation matches what the terminal displays.
	let tab_spaces = " ".repeat(rho_text::TAB_WIDTH);
	let expanded = highlighted.replace('\t', &tab_spaces);

	// Split into lines and ensure each line ends with a color reset.
	// highlight_code may wrap ANSI escapes around newline characters, so
	// after splitting the color from one line can leak into the padding
	// and borders of the output block.
	let all_lines: Vec<String> = expanded
		.lines()
		.map(|line| format!("{line}\x1b[39m"))
		.collect();
	let all_refs: Vec<&str> = all_lines.iter().map(String::as_str).collect();

	let max = if opts.expanded {
		usize::MAX
	} else {
		opts.code_max_lines
	};
	let mut collapsed = collapse_lines(&all_refs, max, theme);

	// Ensure at least one empty line so side borders are visible even for
	// empty content (e.g. empty files).
	if collapsed.is_empty() {
		collapsed.push(String::new());
	}

	let mut sections = vec![OutputSection { label: None, lines: collapsed }];

	if !opts.output_lines.is_empty() {
		sections.push(OutputSection {
			label: Some(theme.dim("Output")),
			lines: opts.output_lines.clone(),
		});
	}

	let header_width = rho_text::width::visible_width_str(&opts.title);
	let block_opts = OutputBlockOptions {
		header: opts.title.clone(),
		header_width,
		state: opts.state,
		sections,
		border_style: make_border_style(theme, opts.state),
		bg_style: make_bg_style(theme, opts.state),
	};
	render_output_block(&block_opts, width)
}

// ── BashRenderer ────────────────────────────────────────────────────────

pub struct BashRenderer;

impl ToolRenderer for BashRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let command = arg_str(args, "command");
		let header_text = theme.fg(ThemeColor::ToolTitle, &theme.bold("\u{2b22} Bash"));
		let header_width = rho_text::width::visible_width_str(&header_text);
		let state = OutputBlockState::Running;
		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections: vec![OutputSection {
				label: Some(theme.dim("Command")),
				lines: vec![format!("$ {command}")],
			}],
			border_style: make_border_style(theme, state),
			bg_style: make_bg_style(theme, state),
		};
		render_output_block(&opts, width)
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		render_result_block("Bash", result, Some("Output"), 10, 30, expanded, theme, width)
	}

	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let command = arg_str(args, "command");
		let call_sections = vec![OutputSection {
			label: Some(theme.dim("Command")),
			lines: vec![format!("$ {command}")],
		}];
		render_combined_block(
			"Bash",
			call_sections,
			result,
			Some("Output"),
			10,
			30,
			expanded,
			theme,
			width,
		)
	}
}

// ── ReadRenderer ────────────────────────────────────────────────────────

pub struct ReadRenderer;

impl ToolRenderer for ReadRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, _width: u16) -> Vec<String> {
		let file = read_path(args);
		let text = theme.fg(ThemeColor::Dim, &format!("\u{2022} Read {file}"));
		vec![format!("  {text}")]
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		if result.is_error {
			let icon = theme.fg(ThemeColor::Error, "\u{2718}");
			let text = theme.fg(ThemeColor::Dim, "Read failed");
			return vec![format!("  {icon} {text}")];
		}
		let icon = theme.fg(ThemeColor::Success, "\u{2714}");
		let title = theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("{icon} Read")));
		render_code_cell(
			&CodeCellOptions {
				code: &result.content,
				language: None,
				title,
				state: OutputBlockState::Success,
				output_lines: vec![],
				expanded,
				code_max_lines: 12,
			},
			theme,
			width,
		)
	}

	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let file = read_path(args);
		if result.is_error {
			let icon = theme.fg(ThemeColor::Error, "\u{2718}");
			let path_display = theme.fg(ThemeColor::Dim, file);
			return vec![format!("  {icon} {path_display}")];
		}
		let icon = theme.fg(ThemeColor::Success, "\u{2714}");
		let path_styled = theme.fg(ThemeColor::Dim, file);
		let title =
			theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("{icon} Read {path_styled}")));
		let lang = language_from_path(file);
		render_code_cell(
			&CodeCellOptions {
				code: &result.content,
				language: lang,
				title,
				state: OutputBlockState::Success,
				output_lines: vec![],
				expanded,
				code_max_lines: 12,
			},
			theme,
			width,
		)
	}
}

/// A single entry in a Read group.
pub struct ReadGroupEntry {
	pub file_path: String,
	pub is_error:  bool,
}

/// Render a group of consecutive Read tool results as a tree.
///
/// Produces output like:
/// ```text
///   • Read (3)
///     ├─ ✔ src/main.rs
///     ├─ ✔ src/lib.rs
///     └─ ✔ Cargo.toml
/// ```
pub fn render_read_group(
	entries: &[ReadGroupEntry],
	tree: &TreeSymbols,
	theme: &Theme,
) -> Vec<String> {
	let mut lines = Vec::with_capacity(entries.len() + 1);
	let header = theme.fg(ThemeColor::Dim, &format!("\u{2022} Read ({})", entries.len()));
	lines.push(format!("  {header}"));
	for (i, entry) in entries.iter().enumerate() {
		let connector = if i == entries.len() - 1 {
			tree.last
		} else {
			tree.branch
		};
		let connector_styled = theme.fg(ThemeColor::Dim, connector);
		let icon = if entry.is_error {
			theme.fg(ThemeColor::Error, "\u{2718}")
		} else {
			theme.fg(ThemeColor::Success, "\u{2714}")
		};
		let path_display = theme.fg(ThemeColor::Dim, &entry.file_path);
		lines.push(format!("    {connector_styled} {icon} {path_display}"));
	}
	lines
}

// ── WriteRenderer ───────────────────────────────────────────────────────

pub struct WriteRenderer;

impl ToolRenderer for WriteRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let file = arg_str(args, "file_path");
		let header_text =
			theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("\u{2b22} Write: {file}")));
		let header_width = rho_text::width::visible_width_str(&header_text);
		let state = OutputBlockState::Running;
		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections: vec![],
			border_style: make_border_style(theme, state),
			bg_style: make_bg_style(theme, state),
		};
		render_output_block(&opts, width)
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		render_result_block("Write", result, None, 5, 20, expanded, theme, width)
	}

	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let file = arg_str(args, "file_path");
		let call_sections =
			vec![OutputSection { label: Some(theme.dim("File")), lines: vec![file.to_owned()] }];
		render_combined_block("Write", call_sections, result, None, 5, 20, expanded, theme, width)
	}
}

// ── EditRenderer ────────────────────────────────────────────────────────

pub struct EditRenderer;

impl ToolRenderer for EditRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let file = arg_str(args, "file_path");
		let header_text =
			theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("\u{2b22} Edit: {file}")));
		let header_width = rho_text::width::visible_width_str(&header_text);
		let state = OutputBlockState::Running;

		let mut sections = Vec::new();
		let old_string = arg_str(args, "old_string");
		let new_string = arg_str(args, "new_string");
		if !old_string.is_empty() || !new_string.is_empty() {
			let mut diff_lines = Vec::new();
			for line in old_string.lines() {
				diff_lines.push(theme.fg(ThemeColor::ToolDiffRemoved, &format!("- {line}")));
			}
			for line in new_string.lines() {
				diff_lines.push(theme.fg(ThemeColor::ToolDiffAdded, &format!("+ {line}")));
			}
			sections.push(OutputSection { label: Some(theme.dim("Diff")), lines: diff_lines });
		}

		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections,
			border_style: make_border_style(theme, state),
			bg_style: make_bg_style(theme, state),
		};
		render_output_block(&opts, width)
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		render_result_block("Edit", result, None, 5, 20, expanded, theme, width)
	}

	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let file = arg_str(args, "file_path");
		let mut call_sections =
			vec![OutputSection { label: Some(theme.dim("File")), lines: vec![file.to_owned()] }];
		let old_string = arg_str(args, "old_string");
		let new_string = arg_str(args, "new_string");
		if !old_string.is_empty() || !new_string.is_empty() {
			let mut diff_lines = Vec::new();
			for line in old_string.lines() {
				diff_lines.push(theme.fg(ThemeColor::ToolDiffRemoved, &format!("- {line}")));
			}
			for line in new_string.lines() {
				diff_lines.push(theme.fg(ThemeColor::ToolDiffAdded, &format!("+ {line}")));
			}
			call_sections.push(OutputSection { label: Some(theme.dim("Diff")), lines: diff_lines });
		}
		render_combined_block("Edit", call_sections, result, None, 5, 20, expanded, theme, width)
	}
}

// ── GrepRenderer ────────────────────────────────────────────────────────

pub struct GrepRenderer;

impl ToolRenderer for GrepRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let pattern = arg_str(args, "pattern");
		let header_text =
			theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("\u{2b22} Grep: {pattern}")));
		let header_width = rho_text::width::visible_width_str(&header_text);
		let state = OutputBlockState::Running;
		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections: vec![],
			border_style: make_border_style(theme, state),
			bg_style: make_bg_style(theme, state),
		};
		render_output_block(&opts, width)
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		render_result_block("Grep", result, Some("Matches"), 5, 20, expanded, theme, width)
	}

	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let pattern = arg_str(args, "pattern");
		let call_sections =
			vec![OutputSection { label: Some(theme.dim("Pattern")), lines: vec![pattern.to_owned()] }];
		render_combined_block(
			"Grep",
			call_sections,
			result,
			Some("Matches"),
			5,
			20,
			expanded,
			theme,
			width,
		)
	}
}

// ── FindRenderer ────────────────────────────────────────────────────────

pub struct FindRenderer;

impl ToolRenderer for FindRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let pattern = arg_str(args, "pattern");
		let header_text =
			theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("\u{2b22} Find: {pattern}")));
		let header_width = rho_text::width::visible_width_str(&header_text);
		let state = OutputBlockState::Running;
		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections: vec![],
			border_style: make_border_style(theme, state),
			bg_style: make_bg_style(theme, state),
		};
		render_output_block(&opts, width)
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		render_result_block("Find", result, Some("Files"), 5, 20, expanded, theme, width)
	}

	fn render_combined(
		&self,
		args: &Value,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		let pattern = arg_str(args, "pattern");
		let call_sections =
			vec![OutputSection { label: Some(theme.dim("Pattern")), lines: vec![pattern.to_owned()] }];
		render_combined_block(
			"Find",
			call_sections,
			result,
			Some("Files"),
			5,
			20,
			expanded,
			theme,
			width,
		)
	}
}

// ── DefaultRenderer ─────────────────────────────────────────────────────

pub struct DefaultRenderer {
	pub name: String,
}

impl ToolRenderer for DefaultRenderer {
	fn render_call(&self, _args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let header_text =
			theme.fg(ThemeColor::ToolTitle, &theme.bold(&format!("\u{2b22} {}", self.name)));
		let header_width = rho_text::width::visible_width_str(&header_text);
		let state = OutputBlockState::Running;
		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections: vec![],
			border_style: make_border_style(theme, state),
			bg_style: make_bg_style(theme, state),
		};
		render_output_block(&opts, width)
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		expanded: bool,
		theme: &Theme,
		width: u16,
	) -> Vec<String> {
		render_result_block(&self.name, result, None, 5, 20, expanded, theme, width)
	}
}

// ── Factory ─────────────────────────────────────────────────────────────

/// Return the appropriate renderer for a tool name.
pub fn get_tool_renderer(tool_name: &str) -> Box<dyn ToolRenderer> {
	match tool_name {
		"Bash" | "bash" => Box::new(BashRenderer),
		"Read" | "read" => Box::new(ReadRenderer),
		"Write" | "write" => Box::new(WriteRenderer),
		"Edit" | "edit" => Box::new(EditRenderer),
		"Grep" | "grep" => Box::new(GrepRenderer),
		"Glob" | "glob" | "Find" | "find" => Box::new(FindRenderer),
		_ => Box::new(DefaultRenderer { name: tool_name.to_owned() }),
	}
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
	use rho_tui::theme::ColorMode;

	use super::*;

	fn test_theme() -> Theme {
		Theme::dark_with_mode(ColorMode::TrueColor)
	}

	fn success_result() -> ToolResultDisplay {
		ToolResultDisplay { content: "operation completed successfully".to_owned(), is_error: false }
	}

	fn error_result() -> ToolResultDisplay {
		ToolResultDisplay { content: "command not found: foobar".to_owned(), is_error: true }
	}

	// ── BashRenderer ────────────────────────────────────────────────

	#[test]
	fn bash_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({ "command": "ls -la" });
		let lines = BashRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "BashRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Bash")), "output should mention Bash",);
	}

	#[test]
	fn bash_render_result_success() {
		let theme = test_theme();
		let lines = BashRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty(), "BashRenderer::render_result should produce output");
	}

	#[test]
	fn bash_render_result_error() {
		let theme = test_theme();
		let lines = BashRenderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty(), "BashRenderer::render_result (error) should produce output");
	}

	#[test]
	fn bash_render_result_collapse_long_output() {
		let theme = test_theme();
		let long_content = (0..30)
			.map(|i| format!("line {i}"))
			.collect::<Vec<_>>()
			.join("\n");
		let result = ToolResultDisplay { content: long_content, is_error: false };
		let lines = BashRenderer.render_result(&result, false, &theme, 80);
		assert!(
			lines.iter().any(|l| l.contains("more lines")),
			"long output should be collapsed with 'more lines' indicator",
		);
	}

	#[test]
	fn bash_render_result_expanded_shows_more_lines() {
		let theme = test_theme();
		// Use 15 lines: collapsed (max 10) should truncate, expanded (max 30) should
		// not.
		let content = (0..15)
			.map(|i| format!("line {i}"))
			.collect::<Vec<_>>()
			.join("\n");
		let result = ToolResultDisplay { content, is_error: false };
		let collapsed = BashRenderer.render_result(&result, false, &theme, 80);
		let expanded = BashRenderer.render_result(&result, true, &theme, 80);
		assert!(
			collapsed.iter().any(|l| l.contains("more lines")),
			"collapsed output should truncate 15 lines at limit 10",
		);
		assert!(
			!expanded.iter().any(|l| l.contains("more lines")),
			"expanded output should show all 15 lines without truncation",
		);
	}

	// ── ReadRenderer ────────────────────────────────────────────────

	#[test]
	fn read_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({ "path": "src/main.rs" });
		let lines = ReadRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "ReadRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Read")), "output should mention Read",);
		assert!(lines.iter().any(|l| l.contains("src/main.rs")), "output should show file path",);
	}

	#[test]
	fn read_render_result_success() {
		let theme = test_theme();
		let lines = ReadRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(lines.len() > 1, "ReadRenderer::render_result should produce a bordered block");
		assert!(
			lines.iter().any(|l| l.contains("\u{2714}")),
			"success result should contain check mark",
		);
		// Should have border chars
		assert!(
			lines.iter().any(|l| l.contains('\u{256d}')),
			"success result should have top border",
		);
	}

	#[test]
	fn read_render_result_error() {
		let theme = test_theme();
		let lines = ReadRenderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty());
		assert!(
			lines.iter().any(|l| l.contains("\u{2718}")),
			"error result should contain cross mark",
		);
		// Errors stay inline — single line, no border
		assert_eq!(lines.len(), 1, "error result should be a single inline line");
	}

	// ── WriteRenderer ───────────────────────────────────────────────

	#[test]
	fn write_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({ "file_path": "output.txt" });
		let lines = WriteRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "WriteRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Write")), "output should mention Write",);
	}

	#[test]
	fn write_render_result_success() {
		let theme = test_theme();
		let lines = WriteRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	#[test]
	fn write_render_result_error() {
		let theme = test_theme();
		let lines = WriteRenderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	// ── EditRenderer ────────────────────────────────────────────────

	#[test]
	fn edit_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({
			"file_path": "lib.rs",
			"old_string": "fn old() {}",
			"new_string": "fn new() {}"
		});
		let lines = EditRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "EditRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Edit")), "output should mention Edit",);
	}

	#[test]
	fn edit_render_call_no_diff() {
		let theme = test_theme();
		let args = serde_json::json!({ "file_path": "lib.rs" });
		let lines = EditRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "EditRenderer::render_call with no diff should still render");
	}

	#[test]
	fn edit_render_result_success() {
		let theme = test_theme();
		let lines = EditRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	#[test]
	fn edit_render_result_error() {
		let theme = test_theme();
		let lines = EditRenderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	// ── GrepRenderer ────────────────────────────────────────────────

	#[test]
	fn grep_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({ "pattern": "TODO" });
		let lines = GrepRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "GrepRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Grep")), "output should mention Grep",);
	}

	#[test]
	fn grep_render_result_success() {
		let theme = test_theme();
		let lines = GrepRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	#[test]
	fn grep_render_result_error() {
		let theme = test_theme();
		let lines = GrepRenderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	// ── FindRenderer ────────────────────────────────────────────────

	#[test]
	fn find_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({ "pattern": "*.rs" });
		let lines = FindRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "FindRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Find")), "output should mention Find",);
	}

	#[test]
	fn find_render_result_success() {
		let theme = test_theme();
		let lines = FindRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	#[test]
	fn find_render_result_error() {
		let theme = test_theme();
		let lines = FindRenderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	// ── DefaultRenderer ─────────────────────────────────────────────

	#[test]
	fn default_render_call_non_empty() {
		let theme = test_theme();
		let renderer = DefaultRenderer { name: "CustomTool".to_owned() };
		let args = serde_json::json!({});
		let lines = renderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "DefaultRenderer::render_call should produce output");
		assert!(
			lines.iter().any(|l| l.contains("CustomTool")),
			"output should mention the tool name",
		);
	}

	#[test]
	fn default_render_result_success() {
		let theme = test_theme();
		let renderer = DefaultRenderer { name: "CustomTool".to_owned() };
		let lines = renderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	#[test]
	fn default_render_result_error() {
		let theme = test_theme();
		let renderer = DefaultRenderer { name: "CustomTool".to_owned() };
		let lines = renderer.render_result(&error_result(), false, &theme, 80);
		assert!(!lines.is_empty());
	}

	// ── Factory ─────────────────────────────────────────────────────

	#[test]
	fn get_tool_renderer_known_tools() {
		for name in &[
			"Bash", "bash", "Read", "read", "Write", "write", "Edit", "edit", "Grep", "grep", "Glob",
			"glob", "Find", "find",
		] {
			let renderer = get_tool_renderer(name);
			let theme = test_theme();
			let args = serde_json::json!({});
			let lines = renderer.render_call(&args, &theme, 80);
			assert!(!lines.is_empty(), "get_tool_renderer({name}) should produce output");
		}
	}

	#[test]
	fn get_tool_renderer_unknown_returns_default() {
		let renderer = get_tool_renderer("UnknownTool");
		let theme = test_theme();
		let args = serde_json::json!({});
		let lines = renderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty());
		assert!(
			lines.iter().any(|l| l.contains("UnknownTool")),
			"unknown tool should show its name via DefaultRenderer",
		);
	}

	// ── render_combined ────────────────────────────────────────────

	#[test]
	fn bash_render_combined_has_command_and_output() {
		let theme = test_theme();
		let args = serde_json::json!({ "command": "ls -la" });
		let result = success_result();
		let lines = BashRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(!lines.is_empty());
		assert!(lines.iter().any(|l| l.contains("Bash")), "combined block should mention Bash");
		assert!(lines.iter().any(|l| l.contains("$ ls -la")), "combined block should show command",);
		assert!(
			lines.iter().any(|l| l.contains("operation completed")),
			"combined block should show result content",
		);
	}

	#[test]
	fn bash_render_combined_error() {
		let theme = test_theme();
		let args = serde_json::json!({ "command": "bad" });
		let result = error_result();
		let lines = BashRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(lines.iter().any(|l| l.contains("\u{2718}")), "error combined should show cross");
	}

	#[test]
	fn write_render_combined_has_file() {
		let theme = test_theme();
		let args = serde_json::json!({ "file_path": "output.txt" });
		let result = success_result();
		let lines = WriteRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(lines.iter().any(|l| l.contains("Write")), "combined block should mention Write");
		assert!(
			lines.iter().any(|l| l.contains("output.txt")),
			"combined block should show file path",
		);
	}

	#[test]
	fn edit_render_combined_has_file_and_diff() {
		let theme = test_theme();
		let args = serde_json::json!({
			"file_path": "lib.rs",
			"old_string": "fn old() {}",
			"new_string": "fn new() {}"
		});
		let result = success_result();
		let lines = EditRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(lines.iter().any(|l| l.contains("Edit")), "combined block should mention Edit");
		assert!(lines.iter().any(|l| l.contains("lib.rs")), "combined block should show file path");
		assert!(
			lines.iter().any(|l| l.contains("- fn old()")),
			"combined block should show removed lines",
		);
		assert!(
			lines.iter().any(|l| l.contains("+ fn new()")),
			"combined block should show added lines",
		);
	}

	#[test]
	fn grep_render_combined_has_pattern_and_matches() {
		let theme = test_theme();
		let args = serde_json::json!({ "pattern": "TODO" });
		let result = success_result();
		let lines = GrepRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(lines.iter().any(|l| l.contains("Grep")), "combined block should mention Grep");
		assert!(lines.iter().any(|l| l.contains("TODO")), "combined block should show pattern",);
	}

	#[test]
	fn find_render_combined_has_pattern_and_files() {
		let theme = test_theme();
		let args = serde_json::json!({ "pattern": "*.rs" });
		let result = success_result();
		let lines = FindRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(lines.iter().any(|l| l.contains("Find")), "combined block should mention Find");
		assert!(lines.iter().any(|l| l.contains("*.rs")), "combined block should show pattern",);
	}

	#[test]
	fn read_render_combined_shows_code_cell() {
		let theme = test_theme();
		let args = serde_json::json!({ "path": "src/main.rs" });
		let result = ToolResultDisplay {
			content:  "fn main() {\n    println!(\"hello\");\n}".to_owned(),
			is_error: false,
		};
		let combined = ReadRenderer.render_combined(&args, &result, false, &theme, 80);
		assert!(combined.len() > 1, "combined should be a bordered code cell");
		assert!(
			combined.iter().any(|l| l.contains("src/main.rs")),
			"combined should include file path in header",
		);
		assert!(
			combined.iter().any(|l| l.contains("\u{2714}")),
			"combined should include check mark for success",
		);
		// Should contain the code content
		assert!(combined.iter().any(|l| l.contains("main")), "combined should show code content",);
	}

	#[test]
	fn read_render_combined_error_stays_inline() {
		let theme = test_theme();
		let args = serde_json::json!({ "path": "missing.rs" });
		let result = error_result();
		let combined = ReadRenderer.render_combined(&args, &result, false, &theme, 80);
		assert_eq!(combined.len(), 1, "error combined should be a single inline line");
		assert!(combined[0].contains('\u{2718}'), "combined error should include cross mark",);
		assert!(combined[0].contains("missing.rs"), "combined error should include file path",);
	}

	// ── render_code_cell ──────────────────────────────────────────

	#[test]
	fn code_cell_basic_output() {
		let theme = test_theme();
		let title = theme.fg(ThemeColor::ToolTitle, &theme.bold("Test"));
		let lines = render_code_cell(
			&CodeCellOptions {
				code: "let x = 1;",
				language: Some("rust"),
				title,
				state: OutputBlockState::Success,
				output_lines: vec![],
				expanded: false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		assert!(lines.len() >= 3, "code cell should have top border, content, bottom border");
		assert!(lines.iter().any(|l| l.contains('\u{256d}')), "should have top border");
		assert!(lines.iter().any(|l| l.contains('\u{2570}')), "should have bottom border");
	}

	#[test]
	fn code_cell_collapses_at_max_lines() {
		let theme = test_theme();
		let code = (0..20)
			.map(|i| format!("line {i}"))
			.collect::<Vec<_>>()
			.join("\n");
		let title = theme.fg(ThemeColor::ToolTitle, &theme.bold("Test"));
		let collapsed = render_code_cell(
			&CodeCellOptions {
				code:           &code,
				language:       None,
				title:          title.clone(),
				state:          OutputBlockState::Success,
				output_lines:   vec![],
				expanded:       false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		assert!(
			collapsed.iter().any(|l| l.contains("more lines")),
			"collapsed code cell should show 'more lines' indicator",
		);

		let title2 = theme.fg(ThemeColor::ToolTitle, &theme.bold("Test"));
		let expanded = render_code_cell(
			&CodeCellOptions {
				code:           &code,
				language:       None,
				title:          title2,
				state:          OutputBlockState::Success,
				output_lines:   vec![],
				expanded:       true,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		assert!(
			!expanded.iter().any(|l| l.contains("more lines")),
			"expanded code cell should not have 'more lines' indicator",
		);
	}

	#[test]
	fn code_cell_empty_code_still_has_side_borders() {
		let theme = test_theme();
		let title = theme.fg(ThemeColor::ToolTitle, &theme.bold("Test"));
		let lines = render_code_cell(
			&CodeCellOptions {
				code: "",
				language: None,
				title,
				state: OutputBlockState::Success,
				output_lines: vec![],
				expanded: false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		// Must have 3 lines: header + 1 empty content line + footer
		assert_eq!(lines.len(), 3, "empty code should produce header + empty content + footer");
		// All lines should be full width
		for (i, line) in lines.iter().enumerate() {
			let w = rho_text::width::visible_width_str(line);
			assert_eq!(w, 80, "empty code line {i} has width {w} instead of 80");
		}
		// Middle line should have side borders
		assert!(lines[1].contains('\u{2502}'), "empty content line should have side borders");
	}

	#[test]
	fn code_cell_with_output_section() {
		let theme = test_theme();
		let title = theme.fg(ThemeColor::ToolTitle, &theme.bold("Test"));
		let lines = render_code_cell(
			&CodeCellOptions {
				code: "hello",
				language: None,
				title,
				state: OutputBlockState::Success,
				output_lines: vec!["warning: truncated".to_owned()],
				expanded: false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		assert!(
			lines.iter().any(|l| l.contains("warning: truncated")),
			"output section should appear in rendered block",
		);
	}

	#[test]
	fn code_cell_blank_lines_have_full_width_borders() {
		let theme = test_theme();
		// Test with plain text (no highlighting)
		let title = "Test".to_owned();
		let code = "line1\n\nline3\n\nline5";
		let lines = render_code_cell(
			&CodeCellOptions {
				code,
				language: None,
				title,
				state: OutputBlockState::Success,
				output_lines: vec![],
				expanded: false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		for (i, line) in lines.iter().enumerate() {
			let w = rho_text::width::visible_width_str(line);
			assert_eq!(w, 80, "plain line {i} has visible width {w} instead of 80: {line:?}");
		}
		// Must have 7 lines: header + 5 content + footer
		assert_eq!(lines.len(), 7, "should have header + 5 content lines + footer");

		// Test with syntax highlighting (ANSI escapes in content)
		let title2 = "Test".to_owned();
		let rust_code = "fn main() {\n\n    println!(\"hello\");\n\n}";
		let lines2 = render_code_cell(
			&CodeCellOptions {
				code:           rust_code,
				language:       Some("rust"),
				title:          title2,
				state:          OutputBlockState::Success,
				output_lines:   vec![],
				expanded:       false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		for (i, line) in lines2.iter().enumerate() {
			let w = rho_text::width::visible_width_str(line);
			assert_eq!(w, 80, "rust line {i} has visible width {w} instead of 80: {line:?}");
		}
		// Must have 7 lines: header + 5 content + footer
		assert_eq!(lines2.len(), 7, "highlighted should have header + 5 content lines + footer");

		// Test with Read tool format (line numbers + tabs)
		let title3 = "Test".to_owned();
		let read_output = "     1\tfn main() {\n     2\t\n     3\t}";
		let lines3 = render_code_cell(
			&CodeCellOptions {
				code:           read_output,
				language:       Some("rust"),
				title:          title3,
				state:          OutputBlockState::Success,
				output_lines:   vec![],
				expanded:       false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		for (i, line) in lines3.iter().enumerate() {
			let w = rho_text::width::visible_width_str(line);
			assert_eq!(w, 80, "read-format line {i} has visible width {w} instead of 80: {line:?}",);
		}
		// Must have 5 lines: header + 3 content + footer
		assert_eq!(lines3.len(), 5, "read format should have header + 3 content lines + footer");

		// Test with markdown content (the reported issue)
		let title4 = "Test".to_owned();
		let md_code = "# Title\n\nSome text\n\n## Section";
		let lines4 = render_code_cell(
			&CodeCellOptions {
				code:           md_code,
				language:       Some("markdown"),
				title:          title4,
				state:          OutputBlockState::Success,
				output_lines:   vec![],
				expanded:       false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		assert_eq!(lines4.len(), 7, "markdown should have header + 5 content lines + footer");
		for (i, line) in lines4.iter().enumerate() {
			let w = rho_text::width::visible_width_str(line);
			assert_eq!(w, 80, "md line {i} has visible width {w} instead of 80: {line:?}");
		}

		// Test with Read tool format + markdown (line numbers + tabs + markdown)
		let title5 = "Test".to_owned();
		let read_md = "     1\t# Title\n     2\t\n     3\tSome text";
		let lines5 = render_code_cell(
			&CodeCellOptions {
				code:           read_md,
				language:       Some("markdown"),
				title:          title5,
				state:          OutputBlockState::Success,
				output_lines:   vec![],
				expanded:       false,
				code_max_lines: 12,
			},
			&theme,
			80,
		);
		assert_eq!(lines5.len(), 5, "read+md should have header + 3 content lines + footer");
		for (i, line) in lines5.iter().enumerate() {
			let w = rho_text::width::visible_width_str(line);
			assert_eq!(w, 80, "read+md line {i} has visible width {w} instead of 80: {line:?}");
		}
		// Verify no tab characters remain in rendered output
		for (i, line) in lines5.iter().enumerate() {
			assert!(!line.contains('\t'), "line {i} should not contain tab characters: {line:?}",);
		}
	}

	#[test]
	fn default_render_combined_delegates_to_render_result() {
		let theme = test_theme();
		let renderer = DefaultRenderer { name: "CustomTool".to_owned() };
		let args = serde_json::json!({});
		let result = success_result();
		let combined = renderer.render_combined(&args, &result, false, &theme, 80);
		let plain = renderer.render_result(&result, false, &theme, 80);
		assert_eq!(combined, plain, "Default render_combined should delegate to render_result");
	}

	#[test]
	fn bash_render_combined_single_border_block() {
		let theme = test_theme();
		let args = serde_json::json!({ "command": "echo hi" });
		let result = ToolResultDisplay { content: "hi".to_owned(), is_error: false };
		let lines = BashRenderer.render_combined(&args, &result, false, &theme, 80);
		// Count top borders (╭) — should be exactly 1
		let top_borders = lines.iter().filter(|l| l.contains('\u{256d}')).count();
		assert_eq!(top_borders, 1, "combined block should have exactly one top border");
		// Count bottom borders (╰) — should be exactly 1
		let bottom_borders = lines.iter().filter(|l| l.contains('\u{2570}')).count();
		assert_eq!(bottom_borders, 1, "combined block should have exactly one bottom border");
	}

	// ── Read group rendering ──────────────────────────────────────

	fn test_tree() -> TreeSymbols {
		TreeSymbols { branch: "├─", last: "╰─", vertical: "│" }
	}

	#[test]
	fn read_group_renders_tree_structure() {
		let theme = test_theme();
		let entries = vec![
			ReadGroupEntry { file_path: "src/main.rs".to_owned(), is_error: false },
			ReadGroupEntry { file_path: "src/lib.rs".to_owned(), is_error: false },
			ReadGroupEntry { file_path: "Cargo.toml".to_owned(), is_error: false },
		];
		let lines = render_read_group(&entries, &test_tree(), &theme);
		assert_eq!(lines.len(), 4, "header + 3 entries");
		assert!(lines[0].contains("Read (3)"), "header should show count");
		assert!(lines[1].contains("├─"), "intermediate entry should use branch");
		assert!(lines[1].contains("src/main.rs"), "first entry should show file path");
		assert!(lines[3].contains("╰─"), "last entry should use last connector");
		assert!(lines[3].contains("Cargo.toml"), "last entry should show file path");
	}

	#[test]
	fn read_group_single_entry() {
		let theme = test_theme();
		let entries = vec![ReadGroupEntry { file_path: "file.rs".to_owned(), is_error: false }];
		let lines = render_read_group(&entries, &test_tree(), &theme);
		assert_eq!(lines.len(), 2, "header + 1 entry");
		assert!(lines[0].contains("Read (1)"), "header should show count");
		assert!(lines[1].contains("╰─"), "single entry should use last connector");
	}

	#[test]
	fn read_group_with_error() {
		let theme = test_theme();
		let entries = vec![
			ReadGroupEntry { file_path: "good.rs".to_owned(), is_error: false },
			ReadGroupEntry { file_path: "bad.rs".to_owned(), is_error: true },
		];
		let lines = render_read_group(&entries, &test_tree(), &theme);
		assert!(lines[1].contains('\u{2714}'), "success entry should have check mark");
		assert!(lines[2].contains('\u{2718}'), "error entry should have cross mark");
	}
}
