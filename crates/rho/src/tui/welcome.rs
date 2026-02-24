//! Welcome screen component — renders a bordered welcome panel with gradient
//! logo.
//!
//! Displays a two-column layout at wide widths (>= 60 chars): the left column
//! shows a greeting, gradient rho logo, and model name; the right column shows
//! tips and recent sessions.  Falls back to a single (left) column at narrow
//! widths.

use std::{fmt::Write as _, rc::Rc};

use rho_tui::{
	component::{Component, InputResult},
	theme::{Theme, ThemeColor},
};

// ── Constants ───────────────────────────────────────────────────────

/// Maximum box width in terminal columns.
const MAX_BOX_WIDTH: usize = 100;

/// Minimum width for the two-column layout (left + divider + right).
const TWO_COL_MIN: usize = 60;

/// 256-color palette indices for the magenta-to-cyan gradient.
const GRADIENT_COLORS: [u8; 6] = [199, 171, 135, 99, 75, 51];

/// Five-line rho logo (block art).
const LOGO_LINES: [&str; 5] = [
	"   \u{2584}\u{2588}\u{2588}\u{2588}\u{2588}\u{2584}",
	"  \u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}",
	"  \u{2588}\u{2588}\u{2588}  \u{2588}\u{2588}\u{2588}",
	"  \u{2580}\u{2588}\u{2588}\u{2588}\u{2588}\u{2580}",
	"  \u{2588}\u{2588}\u{2588}",
];

// ── Component ───────────────────────────────────────────────────────

/// Welcome screen displayed above the chat view on startup.
pub struct WelcomeComponent {
	theme:           Rc<Theme>,
	version:         String,
	model_name:      String,
	recent_sessions: Vec<(String, String)>, // (name, time_ago)
}

impl WelcomeComponent {
	pub const fn new(
		theme: Rc<Theme>,
		version: String,
		model_name: String,
		recent_sessions: Vec<(String, String)>,
	) -> Self {
		Self { theme, version, model_name, recent_sessions }
	}
}

// ── Component trait ─────────────────────────────────────────────────

impl Component for WelcomeComponent {
	fn render(&mut self, width: u16) -> Vec<String> {
		let width = width as usize;
		let box_width = width.min(MAX_BOX_WIDTH);

		// Need at least 20 columns to render anything meaningful.
		if box_width < 20 {
			return Vec::new();
		}

		let inner_width = box_width.saturating_sub(4); // 2 border + 2 padding

		let two_col = inner_width >= TWO_COL_MIN;
		let (left_width, right_width) = if two_col {
			let left = inner_width / 2;
			let right = inner_width - left - 3; // 3 for " | " divider
			(left, right)
		} else {
			(inner_width, 0)
		};

		let left_lines = build_left_column(&self.theme, &self.model_name, left_width);
		let right_lines = if two_col {
			build_right_column(&self.theme, &self.recent_sessions, right_width)
		} else {
			Vec::new()
		};

		let row_count = left_lines.len().max(right_lines.len());

		// Assemble rows inside the border.
		let mut output = Vec::with_capacity(row_count + 2); // +2 for top/bottom border

		// Top border: ╭── rho v{version} ──...──╮
		output.push(build_top_border(&self.theme, &self.version, box_width));

		for i in 0..row_count {
			let left = left_lines.get(i).map_or("", String::as_str);
			let right = right_lines.get(i).map_or("", String::as_str);

			let left_vis = rho_text::width::visible_width_str(left);
			let left_pad = left_width.saturating_sub(left_vis);

			let mut row = String::with_capacity(box_width + 64); // extra for ANSI
			row.push_str(&self.theme.fg(ThemeColor::Border, "\u{2502}"));
			row.push(' ');
			row.push_str(left);
			row.push_str(&" ".repeat(left_pad));

			if two_col {
				row.push_str(&self.theme.fg(ThemeColor::BorderMuted, " \u{2502} "));
				let right_vis = rho_text::width::visible_width_str(right);
				let right_pad = right_width.saturating_sub(right_vis);
				row.push_str(right);
				row.push_str(&" ".repeat(right_pad));
			}

			row.push(' ');
			row.push_str(&self.theme.fg(ThemeColor::Border, "\u{2502}"));
			output.push(row);
		}

		// Bottom border: ╰──...──╯
		output.push(build_bottom_border(&self.theme, box_width));

		output
	}

	fn handle_input(&mut self, _data: &str) -> InputResult {
		InputResult::Ignored
	}
}

// ── Border builders ─────────────────────────────────────────────────

fn build_top_border(theme: &Theme, version: &str, box_width: usize) -> String {
	let label = format!(" rho v{version} ");
	let label_vis = rho_text::width::visible_width_str(&label);

	// ╭── label ──...──╮
	// 2 for ╭/╮, 2 for ── before label
	let right_dashes = box_width.saturating_sub(2 + 2 + label_vis);
	let border = format!(
		"\u{256d}\u{2500}\u{2500}{}{}\u{256e}",
		theme.bold(&label),
		"\u{2500}".repeat(right_dashes),
	);
	theme.fg(ThemeColor::Border, &border)
}

fn build_bottom_border(theme: &Theme, box_width: usize) -> String {
	let inner = "\u{2500}".repeat(box_width.saturating_sub(2));
	let border = format!("\u{2570}{inner}\u{256f}");
	theme.fg(ThemeColor::Border, &border)
}

// ── Left column ─────────────────────────────────────────────────────

fn build_left_column(theme: &Theme, model_name: &str, _col_width: usize) -> Vec<String> {
	let mut lines = Vec::with_capacity(10);

	// Blank line
	lines.push(String::new());

	// "Welcome back!" in bold
	lines.push(theme.bold("Welcome back!"));

	// Blank line
	lines.push(String::new());

	// Gradient logo
	for logo_line in &LOGO_LINES {
		lines.push(apply_gradient(logo_line));
	}

	// Blank line
	lines.push(String::new());

	// Model name (muted)
	lines.push(theme.fg(ThemeColor::Muted, model_name));

	// Provider (muted + dim)
	lines.push(theme.fg(ThemeColor::Dim, "Anthropic"));

	// Blank line
	lines.push(String::new());

	lines
}

// ── Right column ────────────────────────────────────────────────────

fn build_right_column(
	theme: &Theme,
	recent_sessions: &[(String, String)],
	_col_width: usize,
) -> Vec<String> {
	let mut lines = Vec::with_capacity(12);

	// Blank line (align with left)
	lines.push(String::new());

	// Tips heading
	lines.push(theme.bold("Tips"));

	// Tip entries: accent key + muted description
	let tips: [(&str, &str); 4] = [
		("?", " for keyboard shortcuts"),
		("/", " for commands"),
		("!", " to run bash"),
		("!!", " to run bash and send output to LLM"),
	];
	for (key, desc) in &tips {
		let styled =
			format!("{}{}", theme.fg(ThemeColor::Accent, key), theme.fg(ThemeColor::Muted, desc),);
		lines.push(styled);
	}

	// Separator
	lines.push(theme.fg(
		ThemeColor::BorderMuted,
		"\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
	));

	// Recent sessions heading
	lines.push(theme.bold("Recent sessions"));

	if recent_sessions.is_empty() {
		lines.push(theme.fg(ThemeColor::Dim, "(no recent sessions)"));
	} else {
		for (name, time_ago) in recent_sessions {
			let entry =
				format!("\u{2022} {} {}", name, theme.fg(ThemeColor::Dim, &format!("({time_ago})")),);
			lines.push(entry);
		}
	}

	// Pad to match left column height.
	lines.push(String::new());

	lines
}

// ── Gradient ────────────────────────────────────────────────────────

/// Apply a magenta-to-cyan 256-color gradient across visible characters.
fn apply_gradient(line: &str) -> String {
	let chars: Vec<char> = line.chars().collect();
	let total = chars.len();
	if total == 0 {
		return String::new();
	}

	let mut out = String::with_capacity(total * 16);
	for (i, &ch) in chars.iter().enumerate() {
		if ch == ' ' {
			out.push(' ');
			continue;
		}
		let ratio = if total <= 1 {
			0.0
		} else {
			i as f64 / (total - 1) as f64
		};
		let idx = (ratio * (GRADIENT_COLORS.len() - 1) as f64).round() as usize;
		let color = GRADIENT_COLORS[idx.min(GRADIENT_COLORS.len() - 1)];
		let _ = write!(out, "\x1b[38;5;{color}m{ch}\x1b[39m");
	}
	out
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
	use rho_tui::theme::ColorMode;

	use super::*;

	fn test_theme() -> Rc<Theme> {
		Rc::new(Theme::dark_with_mode(ColorMode::TrueColor))
	}

	fn make_component(sessions: Vec<(String, String)>) -> WelcomeComponent {
		WelcomeComponent::new(
			test_theme(),
			"12.9.0".to_owned(),
			"claude-sonnet-4-5".to_owned(),
			sessions,
		)
	}

	#[test]
	fn test_render_contains_version() {
		let mut comp = make_component(vec![]);
		let lines = comp.render(100);
		let joined = lines.join("\n");
		assert!(joined.contains("12.9.0"), "should contain version string");
	}

	#[test]
	fn test_render_contains_welcome_back() {
		let mut comp = make_component(vec![]);
		let lines = comp.render(100);
		let joined = lines.join("\n");
		assert!(joined.contains("Welcome back!"), "should contain greeting");
	}

	#[test]
	fn test_render_contains_model_name() {
		let mut comp = make_component(vec![]);
		let lines = comp.render(100);
		let joined = lines.join("\n");
		assert!(joined.contains("claude-sonnet-4-5"), "should contain model name");
	}

	#[test]
	fn test_render_has_borders() {
		let mut comp = make_component(vec![]);
		let lines = comp.render(100);
		assert!(!lines.is_empty());
		let first = &lines[0];
		let last = &lines[lines.len() - 1];
		// Top border contains ╭, bottom contains ╰
		assert!(first.contains('\u{256d}'), "top border should have ╭");
		assert!(last.contains('\u{2570}'), "bottom border should have ╰");
	}

	#[test]
	fn test_two_column_layout_at_wide_width() {
		let mut comp = make_component(vec![("my-session".to_owned(), "2h ago".to_owned())]);
		let lines = comp.render(100);
		let joined = lines.join("\n");
		// Two-column mode should render the tips section
		assert!(joined.contains("Tips"), "wide layout should show Tips column");
		assert!(joined.contains("my-session"), "wide layout should show recent sessions");
	}

	#[test]
	fn test_single_column_at_narrow_width() {
		let mut comp = make_component(vec![("my-session".to_owned(), "2h ago".to_owned())]);
		// Width 40 -> inner = 36, below TWO_COL_MIN (60)
		let lines = comp.render(40);
		let joined = lines.join("\n");
		assert!(joined.contains("Welcome back!"), "narrow layout should still show greeting");
		// Tips column should NOT appear
		assert!(!joined.contains("Tips"), "narrow layout should not show Tips");
	}

	#[test]
	fn test_very_narrow_returns_empty() {
		let mut comp = make_component(vec![]);
		let lines = comp.render(10);
		assert!(lines.is_empty(), "extremely narrow width should produce nothing");
	}

	#[test]
	fn test_gradient_applies_color() {
		let result = apply_gradient("ABC");
		// Should contain 256-color escape sequences
		assert!(result.contains("\x1b[38;5;"), "gradient should use 256 colors");
		assert!(result.contains('A'));
		assert!(result.contains('B'));
		assert!(result.contains('C'));
	}

	#[test]
	fn test_gradient_skips_spaces() {
		let result = apply_gradient("A B");
		// Space should remain a plain space (no ANSI wrapping)
		assert!(result.contains(' '));
		// Count the ANSI sequences — only non-space chars get them
		let ansi_count = result.matches("\x1b[38;5;").count();
		assert_eq!(ansi_count, 2, "only non-space chars should be colored");
	}

	#[test]
	fn test_empty_recent_sessions() {
		let mut comp = make_component(vec![]);
		let lines = comp.render(100);
		let joined = lines.join("\n");
		assert!(joined.contains("no recent sessions"), "should show placeholder when no sessions");
	}

	#[test]
	fn test_recent_sessions_listed() {
		let mut comp = make_component(vec![
			("alpha".to_owned(), "1h ago".to_owned()),
			("beta".to_owned(), "3d ago".to_owned()),
		]);
		let lines = comp.render(100);
		let joined = lines.join("\n");
		assert!(joined.contains("alpha"));
		assert!(joined.contains("1h ago"));
		assert!(joined.contains("beta"));
		assert!(joined.contains("3d ago"));
	}
}
