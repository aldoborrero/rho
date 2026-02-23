//! Segment-based status line that produces `EditorTopBorder` content.
//!
//! Each segment (pi, model, path, git, `token_total`) is rendered left-to-right
//! with box-drawing vertical separators. Segments are dropped from the right
//! when the total width exceeds the available space.

use std::rc::Rc;

use rho_tui::{
	components::editor::EditorTopBorder,
	theme::{Theme, ThemeColor},
};

/// Mutable state backing the status line segments.
pub struct StatusLineState {
	pub model_name:     String,
	pub thinking_level: String,
	pub git_branch:     Option<String>,
	pub git_dirty:      bool,
	pub input_tokens:   u32,
	pub output_tokens:  u32,
	pub session_id:     Option<String>,
}

/// Status line component that renders segments into an `EditorTopBorder`.
///
/// This is NOT a `Component` — it is held directly by the app and its output
/// is fed to `Editor::set_top_border`.
pub struct StatusLineComponent {
	theme: Rc<Theme>,
	state: StatusLineState,
}

impl StatusLineComponent {
	pub fn new(theme: Rc<Theme>, model: &str) -> Self {
		Self {
			theme,
			state: StatusLineState {
				model_name:     model.to_owned(),
				thinking_level: String::new(),
				git_branch:     None,
				git_dirty:      false,
				input_tokens:   0,
				output_tokens:  0,
				session_id:     None,
			},
		}
	}

	#[allow(dead_code, reason = "public API — called when model changes at runtime")]
	pub fn set_model(&mut self, name: &str) {
		name.clone_into(&mut self.state.model_name);
	}

	#[allow(dead_code, reason = "public API — called when thinking level changes")]
	pub fn set_thinking_level(&mut self, level: &str) {
		level.clone_into(&mut self.state.thinking_level);
	}

	pub const fn set_usage(&mut self, input: u32, output: u32) {
		self.state.input_tokens = input;
		self.state.output_tokens = output;
	}

	#[allow(dead_code, reason = "public API — called when git state changes")]
	pub fn set_git_branch(&mut self, branch: Option<String>, dirty: bool) {
		self.state.git_branch = branch;
		self.state.git_dirty = dirty;
	}

	pub fn set_session_id(&mut self, id: &str) {
		self.state.session_id = Some(id.to_owned());
	}

	/// Produce an `EditorTopBorder` for the editor's top border.
	///
	/// Segments are rendered left-to-right: `pi │ model │ path │ git │ tokens`.
	/// If all segments exceed `width`, segments are dropped from the right
	/// (tokens first, then git, then path).
	pub fn get_top_border(&self, width: u16) -> EditorTopBorder {
		let theme = &self.theme;
		let sep = theme.fg(ThemeColor::StatusLineSep, " \u{2502} ");
		let sep_width: usize = 3; // " │ " is 3 visible chars

		// Build segments as (styled_text, visible_width) pairs.
		let logo_segment = build_logo_segment(theme);
		let model_segment = build_model_segment(theme, &self.state);
		let path_segment = build_path_segment(theme);
		let git_segment = build_git_segment(theme, &self.state);
		let token_segment = build_token_segment(theme, &self.state);

		// Collect segments that have content.
		let mut segments: Vec<(String, usize)> = Vec::new();
		segments.push(logo_segment);
		segments.push(model_segment);
		if let Some(seg) = path_segment {
			segments.push(seg);
		}
		if let Some(seg) = git_segment {
			segments.push(seg);
		}
		if let Some(seg) = token_segment {
			segments.push(seg);
		}

		// Calculate total visible width including separators.
		let max_width = width as usize;

		// Drop segments from the right until they fit.
		while segments.len() > 2 {
			let total = total_width(&segments, sep_width);
			if total <= max_width {
				break;
			}
			segments.pop();
		}

		// Join with separator.
		let content: String = segments
			.iter()
			.map(|(text, _)| text.as_str())
			.collect::<Vec<_>>()
			.join(&sep);
		let visible = total_width(&segments, sep_width);

		EditorTopBorder { content, width: visible }
	}
}

// ── Segment builders ───────────────────────────────────────────────

fn build_logo_segment(theme: &Theme) -> (String, usize) {
	let text = "\u{03c1}"; // ρ
	let styled = theme.fg(ThemeColor::Accent, text);
	(styled, 1)
}

fn build_model_segment(theme: &Theme, state: &StatusLineState) -> (String, usize) {
	let text = if state.thinking_level.is_empty() {
		state.model_name.clone()
	} else {
		format!("{} ({})", state.model_name, state.thinking_level)
	};
	let width = rho_text::width::visible_width_str(&text);
	let styled = theme.fg(ThemeColor::StatusLineModel, &text);
	(styled, width)
}

fn build_path_segment(theme: &Theme) -> Option<(String, usize)> {
	let cwd = std::env::current_dir().ok()?;
	let path_str = cwd.to_string_lossy();

	// Try to replace home directory with ~
	let display_path = if let Some(home) = dirs::home_dir() {
		let home_str = home.to_string_lossy();
		if path_str.starts_with(home_str.as_ref()) {
			format!("~{}", &path_str[home_str.len()..])
		} else {
			path_str.to_string()
		}
	} else {
		path_str.to_string()
	};

	let abbreviated = abbreviate_path(&display_path, 40);
	let width = rho_text::width::visible_width_str(&abbreviated);
	let styled = theme.fg(ThemeColor::StatusLinePath, &abbreviated);
	Some((styled, width))
}

fn build_git_segment(theme: &Theme, state: &StatusLineState) -> Option<(String, usize)> {
	let branch = state.git_branch.as_deref()?;
	let dirty_indicator = if state.git_dirty { " *" } else { "" };
	let text = format!("{branch}{dirty_indicator}");
	let width = rho_text::width::visible_width_str(&text);
	let color = if state.git_dirty {
		ThemeColor::StatusLineGitDirty
	} else {
		ThemeColor::StatusLineGitClean
	};
	let styled = theme.fg(color, &text);
	Some((styled, width))
}

fn build_token_segment(theme: &Theme, state: &StatusLineState) -> Option<(String, usize)> {
	let total = state.input_tokens + state.output_tokens;
	if total == 0 {
		return None;
	}
	let text = format!(
		"\u{2191}{} \u{2193}{}",
		format_tokens(state.input_tokens),
		format_tokens(state.output_tokens),
	);
	let width = rho_text::width::visible_width_str(&text);
	let styled = theme.fg(ThemeColor::StatusLineOutput, &text);
	Some((styled, width))
}

// ── Helpers ────────────────────────────────────────────────────────

/// Abbreviate a path to at most `max_len` visible characters.
/// If truncation is needed, the path is prefixed with "…" (U+2026).
/// Matches the TypeScript `pathSegment` implementation in segments.ts.
fn abbreviate_path(path: &str, max_len: usize) -> String {
	let char_count = path.chars().count();
	if char_count <= max_len {
		return path.to_owned();
	}
	let ellipsis = '\u{2026}'; // …
	let slice_len = max_len.saturating_sub(1); // 1 char for ellipsis
	let suffix: String = path.chars().skip(char_count - slice_len).collect();
	format!("{ellipsis}{suffix}")
}

/// Format a token count as a compact human-readable string.
fn format_tokens(n: u32) -> String {
	if n < 1_000 {
		n.to_string()
	} else if n < 10_000 {
		// e.g. 1500 -> "1.5k"
		let whole = n / 1_000;
		let frac = (n % 1_000) / 100;
		if frac == 0 {
			format!("{whole}k")
		} else {
			format!("{whole}.{frac}k")
		}
	} else if n < 1_000_000 {
		// e.g. 12345 -> "12k"
		format!("{}k", n / 1_000)
	} else {
		// e.g. 1_500_000 -> "1.5M"
		let whole = n / 1_000_000;
		let frac = (n % 1_000_000) / 100_000;
		if frac == 0 {
			format!("{whole}M")
		} else {
			format!("{whole}.{frac}M")
		}
	}
}

/// Calculate the total visible width of segments joined by separators.
fn total_width(segments: &[(String, usize)], sep_width: usize) -> usize {
	if segments.is_empty() {
		return 0;
	}
	let content_width: usize = segments.iter().map(|(_, w)| w).sum();
	let sep_total = sep_width * (segments.len() - 1);
	content_width + sep_total
}

#[cfg(test)]
mod tests {
	use rho_tui::theme::ColorMode;

	use super::*;

	fn test_theme() -> Rc<Theme> {
		Rc::new(Theme::dark_with_mode(ColorMode::TrueColor))
	}

	#[test]
	fn test_get_top_border_produces_non_empty_content() {
		let theme = test_theme();
		let status = StatusLineComponent::new(Rc::clone(&theme), "claude-sonnet-4");
		let border = status.get_top_border(80);
		assert!(!border.content.is_empty());
		assert!(border.width > 0);
	}

	#[test]
	fn test_model_update_reflected_in_output() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "old-model");
		let border1 = status.get_top_border(120);

		status.set_model("new-model");
		let border2 = status.get_top_border(120);

		assert!(border1.content.contains("old-model"));
		assert!(border2.content.contains("new-model"));
		assert!(!border2.content.contains("old-model"));
	}

	#[test]
	fn test_thinking_level_in_model_segment() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "claude-sonnet-4");
		status.set_thinking_level("high");
		let border = status.get_top_border(120);
		assert!(border.content.contains("high"));
	}

	#[test]
	fn test_usage_reflected_in_output() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "model");
		status.set_usage(1500, 300);
		let border = status.get_top_border(120);
		// Should contain formatted tokens
		assert!(border.content.contains("1.5k"));
		assert!(border.content.contains("300"));
	}

	#[test]
	fn test_git_branch_reflected_in_output() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "model");
		status.set_git_branch(Some("main".to_owned()), false);
		let border = status.get_top_border(120);
		assert!(border.content.contains("main"));
	}

	#[test]
	fn test_git_dirty_indicator() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "model");
		status.set_git_branch(Some("main".to_owned()), true);
		let border = status.get_top_border(120);
		assert!(border.content.contains("main"));
		assert!(border.content.contains('*'));
	}

	#[test]
	fn test_session_id() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "model");
		status.set_session_id("abc-123");
		assert_eq!(status.state.session_id.as_deref(), Some("abc-123"));
	}

	// ── Token formatting ───────────────────────────────────────────

	#[test]
	fn test_format_tokens_below_1000() {
		assert_eq!(format_tokens(0), "0");
		assert_eq!(format_tokens(500), "500");
		assert_eq!(format_tokens(999), "999");
	}

	#[test]
	fn test_format_tokens_thousands() {
		assert_eq!(format_tokens(1000), "1k");
		assert_eq!(format_tokens(1500), "1.5k");
		assert_eq!(format_tokens(2300), "2.3k");
		assert_eq!(format_tokens(9900), "9.9k");
	}

	#[test]
	fn test_format_tokens_tens_of_thousands() {
		assert_eq!(format_tokens(10_000), "10k");
		assert_eq!(format_tokens(12_345), "12k");
		assert_eq!(format_tokens(99_999), "99k");
	}

	#[test]
	fn test_format_tokens_millions() {
		assert_eq!(format_tokens(1_000_000), "1M");
		assert_eq!(format_tokens(1_500_000), "1.5M");
		assert_eq!(format_tokens(2_300_000), "2.3M");
	}

	// ── Path abbreviation ──────────────────────────────────────────

	#[test]
	fn test_abbreviate_path_short() {
		assert_eq!(abbreviate_path("/home/user", 40), "/home/user");
	}

	#[test]
	fn test_abbreviate_path_exact_max() {
		let path = "a".repeat(40);
		assert_eq!(abbreviate_path(&path, 40), path);
	}

	#[test]
	fn test_abbreviate_path_long() {
		let path = "/home/user/very/long/path/to/some/deeply/nested/directory/structure";
		let result = abbreviate_path(path, 40);
		assert!(result.starts_with('\u{2026}')); // …
		assert_eq!(result.chars().count(), 40);
	}

	// ── Width calculation ──────────────────────────────────────────

	#[test]
	fn test_total_width_empty() {
		assert_eq!(total_width(&[], 3), 0);
	}

	#[test]
	fn test_total_width_single() {
		let segments = vec![("x".to_owned(), 5)];
		assert_eq!(total_width(&segments, 3), 5);
	}

	#[test]
	fn test_total_width_multiple() {
		let segments = vec![("a".to_owned(), 5), ("b".to_owned(), 10), ("c".to_owned(), 3)];
		// 5 + 10 + 3 = 18 content + 2 * 3 = 6 separators = 24
		assert_eq!(total_width(&segments, 3), 24);
	}

	// ── Segment dropping ───────────────────────────────────────────

	#[test]
	fn test_narrow_width_drops_segments() {
		let theme = test_theme();
		let mut status = StatusLineComponent::new(Rc::clone(&theme), "claude-sonnet-4");
		status.set_git_branch(Some("main".to_owned()), false);
		status.set_usage(5000, 1000);

		// Wide enough for all 5 segments.
		let wide = status.get_top_border(120);
		assert!(wide.content.contains("main"));
		assert!(wide.content.contains("5k"));

		// Narrow width — should drop optional segments (tokens, git, path)
		// "ρ │ claude-sonnet-4" needs ~19 chars, so use 25 which fits logo+model
		// but not the extra segments.
		let narrow = status.get_top_border(25);
		assert!(!narrow.content.contains("main"));
		assert!(!narrow.content.contains("5k"));
	}

	#[test]
	fn test_zero_tokens_hides_token_segment() {
		let theme = test_theme();
		let status = StatusLineComponent::new(Rc::clone(&theme), "model");
		let border = status.get_top_border(120);
		// Should not contain arrow characters when no tokens
		assert!(!border.content.contains('\u{2191}'));
		assert!(!border.content.contains('\u{2193}'));
	}
}
