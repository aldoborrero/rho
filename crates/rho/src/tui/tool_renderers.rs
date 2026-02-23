//! Per-tool rendering functions that produce styled ANSI output.
//!
//! Each renderer maps a tool call or tool result into `Vec<String>` of styled
//! lines, using `OutputBlock` for bordered display and inline text for
//! lightweight tools like Read.

use rho_tui::{
	components::output_block::{
		OutputBlockOptions, OutputBlockState, OutputSection, render_output_block,
	},
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
}

// ── Style helpers ───────────────────────────────────────────────────────

fn make_border_style(theme: &Theme, state: OutputBlockState) -> Box<dyn Fn(&str) -> String> {
	let color = match state {
		OutputBlockState::Pending | OutputBlockState::Running => ThemeColor::Accent,
		OutputBlockState::Success => ThemeColor::Dim,
		OutputBlockState::Error | OutputBlockState::Warning => ThemeColor::Error,
	};
	theme.border_color_fn(color)
}

#[allow(clippy::type_complexity, reason = "matches OutputBlockOptions::bg_style signature")]
fn make_bg_style(theme: &Theme, state: OutputBlockState) -> Option<Box<dyn Fn(&str) -> String>> {
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
fn collapse_lines(lines: &[&str], max: usize, theme: &Theme) -> Vec<String> {
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
	let opts = OutputBlockOptions {
		header: header_text,
		header_width,
		state,
		sections: vec![OutputSection {
			label: section_label.map(|l| theme.dim(l)),
			lines: collapsed,
		}],
		border_style: make_border_style(theme, state),
		bg_style: make_bg_style(theme, state),
	};
	render_output_block(&opts, width)
}

// ── BashRenderer ────────────────────────────────────────────────────────

pub struct BashRenderer;

impl ToolRenderer for BashRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, width: u16) -> Vec<String> {
		let command = arg_str(args, "command");
		let header_text = format!(
			"{} {}",
			theme.fg(ThemeColor::ToolTitle, &theme.bold("\u{2b22} Bash")),
			theme.fg(ThemeColor::Dim, &format!("$ {command}")),
		);
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
		render_result_block("Bash", result, Some("Output"), 3, 10, expanded, theme, width)
	}
}

// ── ReadRenderer ────────────────────────────────────────────────────────

pub struct ReadRenderer;

impl ToolRenderer for ReadRenderer {
	fn render_call(&self, args: &Value, theme: &Theme, _width: u16) -> Vec<String> {
		let file = arg_str(args, "file_path");
		let text = theme.fg(ThemeColor::Dim, &format!("\u{2022} Read {file}"));
		vec![format!("  {text}")]
	}

	fn render_result(
		&self,
		result: &ToolResultDisplay,
		_expanded: bool,
		theme: &Theme,
		_width: u16,
	) -> Vec<String> {
		let icon = if result.is_error {
			theme.fg(ThemeColor::Error, "\u{2718}")
		} else {
			theme.fg(ThemeColor::Success, "\u{2714}")
		};
		let label = if result.is_error {
			"Read failed"
		} else {
			"Read"
		};
		let text = theme.fg(ThemeColor::Dim, label);
		vec![format!("  {icon} {text}")]
	}
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
		render_result_block("Write", result, None, 3, 10, expanded, theme, width)
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
		render_result_block("Edit", result, None, 3, 10, expanded, theme, width)
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
		render_result_block("Grep", result, Some("Matches"), 3, 10, expanded, theme, width)
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
		render_result_block("Find", result, Some("Files"), 3, 10, expanded, theme, width)
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
		render_result_block(&self.name, result, None, 3, 10, expanded, theme, width)
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
		let content = (0..8)
			.map(|i| format!("line {i}"))
			.collect::<Vec<_>>()
			.join("\n");
		let result = ToolResultDisplay { content, is_error: false };
		let collapsed = BashRenderer.render_result(&result, false, &theme, 80);
		let expanded = BashRenderer.render_result(&result, true, &theme, 80);
		// 8 lines: collapsed (max 3) should have "more lines", expanded (max 10) should
		// not
		assert!(
			collapsed.iter().any(|l| l.contains("more lines")),
			"collapsed output should truncate 8 lines at limit 3",
		);
		assert!(
			!expanded.iter().any(|l| l.contains("more lines")),
			"expanded output should show all 8 lines without truncation",
		);
	}

	// ── ReadRenderer ────────────────────────────────────────────────

	#[test]
	fn read_render_call_non_empty() {
		let theme = test_theme();
		let args = serde_json::json!({ "file_path": "src/main.rs" });
		let lines = ReadRenderer.render_call(&args, &theme, 80);
		assert!(!lines.is_empty(), "ReadRenderer::render_call should produce output");
		assert!(lines.iter().any(|l| l.contains("Read")), "output should mention Read",);
	}

	#[test]
	fn read_render_result_success() {
		let theme = test_theme();
		let lines = ReadRenderer.render_result(&success_result(), false, &theme, 80);
		assert!(!lines.is_empty(), "ReadRenderer::render_result should produce output");
		assert!(
			lines.iter().any(|l| l.contains("\u{2714}")),
			"success result should contain check mark",
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
}
