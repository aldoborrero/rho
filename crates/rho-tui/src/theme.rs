//! Theme system for consistent TUI styling.
//!
//! Mirrors the TypeScript theme architecture: colors are stored as
//! pre-computed ANSI escape sequences, and styling methods wrap text
//! with appropriate escape codes.

use std::{collections::HashMap, rc::Rc};

use crate::{
	components::{
		editor::{BorderColorFn, HintStyleFn},
		filterable_select::FilterableSelectTheme,
		markdown::MarkdownTheme,
		select_list::SelectListTheme,
		tab_bar::TabBarTheme,
	},
	highlight::HighlightColors,
	symbols::SymbolTheme,
};

/// Foreground color keys (53 variants, matching TypeScript `ThemeColor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeColor {
	// Core UI
	Accent,
	Border,
	BorderAccent,
	BorderMuted,
	Success,
	Error,
	Warning,
	Muted,
	Dim,
	Text,
	ThinkingText,
	// Message text
	UserMessageText,
	CustomMessageText,
	CustomMessageLabel,
	ToolTitle,
	ToolOutput,
	// Markdown
	MdHeading,
	MdLink,
	MdLinkUrl,
	MdCode,
	MdCodeBlock,
	MdCodeBlockBorder,
	MdQuote,
	MdQuoteBorder,
	MdHr,
	MdListBullet,
	// Tool diffs
	ToolDiffAdded,
	ToolDiffRemoved,
	ToolDiffContext,
	// Syntax highlighting
	SyntaxComment,
	SyntaxKeyword,
	SyntaxFunction,
	SyntaxVariable,
	SyntaxString,
	SyntaxNumber,
	SyntaxType,
	SyntaxOperator,
	SyntaxPunctuation,
	// Thinking borders
	ThinkingOff,
	ThinkingMinimal,
	ThinkingLow,
	ThinkingMedium,
	ThinkingHigh,
	ThinkingXhigh,
	// Mode borders
	BashMode,
	PythonMode,
	// Status line
	StatusLineSep,
	StatusLineModel,
	StatusLinePath,
	StatusLineGitClean,
	StatusLineGitDirty,
	StatusLineContext,
	StatusLineSpend,
	StatusLineStaged,
	StatusLineDirty,
	StatusLineUntracked,
	StatusLineOutput,
	StatusLineCost,
	StatusLineSubagents,
}

/// Background color keys (7 variants, matching TypeScript `ThemeBg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeBg {
	SelectedBg,
	UserMessageBg,
	CustomMessageBg,
	ToolPendingBg,
	ToolSuccessBg,
	ToolErrorBg,
	StatusLineBg,
}

/// Detected terminal color capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
	TrueColor,
	Color256,
}

/// A color value: either an RGB hex string or a 256-color index.
#[derive(Debug, Clone)]
pub enum ColorValue {
	Rgb(u8, u8, u8),
	Index256(u8),
}

/// Theme providing consistent styling across all TUI components.
///
/// Colors are pre-computed to ANSI escape sequences at construction time.
/// Styling methods wrap text with appropriate escape codes, using specific
/// attribute resets (not full `\x1b[0m`) to allow composing fg + bg + bold.
pub struct Theme {
	fg_colors: HashMap<ThemeColor, String>,
	bg_colors: HashMap<ThemeBg, String>,
}

impl Theme {
	/// Create a theme from raw color values. Colors are converted to ANSI
	/// escape sequences based on the detected color mode.
	pub fn new(
		fg_values: HashMap<ThemeColor, ColorValue>,
		bg_values: HashMap<ThemeBg, ColorValue>,
		mode: ColorMode,
	) -> Self {
		let fg_colors = fg_values
			.into_iter()
			.map(|(k, v)| (k, Self::fg_ansi_for(&v, mode)))
			.collect();
		let bg_colors = bg_values
			.into_iter()
			.map(|(k, v)| (k, Self::bg_ansi_for(&v, mode)))
			.collect();
		Self { fg_colors, bg_colors }
	}

	/// Create the default dark theme with auto-detected color mode.
	pub fn dark() -> Self {
		let mode = Self::detect_color_mode();
		Self::dark_with_mode(mode)
	}

	/// Create the default dark theme with an explicit color mode.
	pub fn dark_with_mode(mode: ColorMode) -> Self {
		Self::new(dark_fg_colors(), dark_bg_colors(), mode)
	}

	/// Apply foreground color to text: `${ansi}${text}\x1b[39m`
	pub fn fg(&self, color: ThemeColor, text: &str) -> String {
		if let Some(ansi) = self.fg_colors.get(&color) {
			format!("{ansi}{text}\x1b[39m")
		} else {
			text.to_owned()
		}
	}

	/// Apply background color to text: `${ansi}${text}\x1b[49m`
	pub fn bg(&self, color: ThemeBg, text: &str) -> String {
		if let Some(ansi) = self.bg_colors.get(&color) {
			format!("{ansi}{text}\x1b[49m")
		} else {
			text.to_owned()
		}
	}

	/// Bold text.
	pub fn bold(&self, text: &str) -> String {
		format!("\x1b[1m{text}\x1b[22m")
	}

	/// Italic text.
	pub fn italic(&self, text: &str) -> String {
		format!("\x1b[3m{text}\x1b[23m")
	}

	/// Dim text.
	pub fn dim(&self, text: &str) -> String {
		format!("\x1b[2m{text}\x1b[22m")
	}

	/// Underline text.
	pub fn underline(&self, text: &str) -> String {
		format!("\x1b[4m{text}\x1b[24m")
	}

	/// Strikethrough text.
	pub fn strikethrough(&self, text: &str) -> String {
		format!("\x1b[9m{text}\x1b[29m")
	}

	/// Inverse (swap fg/bg) text.
	pub fn inverse(&self, text: &str) -> String {
		format!("\x1b[7m{text}\x1b[27m")
	}

	/// Get the raw ANSI foreground escape sequence for a color.
	pub fn fg_ansi(&self, color: ThemeColor) -> &str {
		self.fg_colors.get(&color).map_or("", String::as_str)
	}

	/// Get the raw ANSI background escape sequence for a color.
	pub fn bg_ansi(&self, color: ThemeBg) -> &str {
		self.bg_colors.get(&color).map_or("", String::as_str)
	}

	fn fg_ansi_for(value: &ColorValue, mode: ColorMode) -> String {
		match (value, mode) {
			(ColorValue::Rgb(r, g, b), ColorMode::TrueColor) => {
				format!("\x1b[38;2;{r};{g};{b}m")
			},
			(ColorValue::Rgb(r, g, b), ColorMode::Color256) => {
				let idx = rgb_to_256(*r, *g, *b);
				format!("\x1b[38;5;{idx}m")
			},
			(ColorValue::Index256(idx), _) => {
				format!("\x1b[38;5;{idx}m")
			},
		}
	}

	fn bg_ansi_for(value: &ColorValue, mode: ColorMode) -> String {
		match (value, mode) {
			(ColorValue::Rgb(r, g, b), ColorMode::TrueColor) => {
				format!("\x1b[48;2;{r};{g};{b}m")
			},
			(ColorValue::Rgb(r, g, b), ColorMode::Color256) => {
				let idx = rgb_to_256(*r, *g, *b);
				format!("\x1b[48;5;{idx}m")
			},
			(ColorValue::Index256(idx), _) => {
				format!("\x1b[48;5;{idx}m")
			},
		}
	}

	/// Build a `MarkdownTheme` using this theme's colors.
	pub fn markdown_theme(&self, symbols: SymbolTheme) -> MarkdownTheme {
		MarkdownTheme {
			heading: Rc::from(self.fg_closure(ThemeColor::MdHeading)),
			link: Rc::from(self.fg_closure(ThemeColor::MdLink)),
			link_url: Rc::from(self.fg_closure(ThemeColor::MdLinkUrl)),
			code: Rc::from(self.fg_closure(ThemeColor::MdCode)),
			code_block: Rc::from(self.fg_closure(ThemeColor::MdCodeBlock)),
			code_block_border: Rc::from(self.fg_closure(ThemeColor::MdCodeBlockBorder)),
			quote: Rc::from(self.fg_closure(ThemeColor::MdQuote)),
			quote_border: Rc::from(self.fg_closure(ThemeColor::MdQuoteBorder)),
			hr: Rc::from(self.fg_closure(ThemeColor::MdHr)),
			list_bullet: Rc::from(self.fg_closure(ThemeColor::MdListBullet)),
			bold: Rc::new(|s: &str| format!("\x1b[1m{s}\x1b[22m")),
			italic: Rc::new(|s: &str| format!("\x1b[3m{s}\x1b[23m")),
			strikethrough: Rc::new(|s: &str| format!("\x1b[9m{s}\x1b[29m")),
			underline: Rc::new(|s: &str| format!("\x1b[4m{s}\x1b[24m")),
			highlight_code: None,
			get_mermaid_image: None,
			symbols,
			highlight_colors: Some(self.highlight_colors()),
		}
	}

	/// Build a `SelectListTheme` using this theme's colors.
	pub fn select_list_theme(&self, symbols: SymbolTheme) -> SelectListTheme {
		SelectListTheme {
			selected_prefix: self.fg_closure(ThemeColor::Accent),
			selected_text: self.fg_closure(ThemeColor::Accent),
			description: self.fg_closure(ThemeColor::Muted),
			scroll_info: self.fg_closure(ThemeColor::Muted),
			no_match: self.fg_closure(ThemeColor::Muted),
			symbols,
		}
	}

	/// Build a `TabBarTheme` using this theme's colors.
	///
	/// Matches oh-my-pi's theming: bold+accent label, bold+selectedBg active
	/// tab, muted inactive tabs.
	pub fn tab_bar_theme(&self) -> TabBarTheme {
		// oh-my-pi: label = bold(fg("accent", text))
		let accent_ansi = self.fg_ansi(ThemeColor::Accent).to_owned();
		let label: Box<dyn Fn(&str) -> String> = if accent_ansi.is_empty() {
			Box::new(|s: &str| format!("\x1b[1m{s}\x1b[22m"))
		} else {
			Box::new(move |s: &str| format!("\x1b[1m{accent_ansi}{s}\x1b[39m\x1b[22m"))
		};

		// oh-my-pi: activeTab = bold(bg("selectedBg", fg("text", text)))
		// "text" color is terminal default (empty), so it's bold + bg.
		let bg_ansi = self.bg_ansi(ThemeBg::SelectedBg).to_owned();
		let active_tab: Box<dyn Fn(&str) -> String> = if bg_ansi.is_empty() {
			Box::new(|s: &str| format!("\x1b[1m{s}\x1b[22m"))
		} else {
			Box::new(move |s: &str| format!("\x1b[1m{bg_ansi}{s}\x1b[49m\x1b[22m"))
		};

		TabBarTheme {
			label,
			active_tab,
			// oh-my-pi: inactiveTab = fg("muted", text)
			inactive_tab: self.fg_closure(ThemeColor::Muted),
			// oh-my-pi: hint = fg("dim", text)
			hint: self.fg_closure(ThemeColor::Dim),
		}
	}

	/// Build a `FilterableSelectTheme` using this theme's colors.
	///
	/// The `select_list_factory` is a closure that produces fresh
	/// `SelectListTheme` instances — needed because the inner `SelectList`
	/// is rebuilt on every filter change and `SelectListTheme` is not `Clone`.
	pub fn filterable_select_theme(&self, symbols: SymbolTheme) -> FilterableSelectTheme {
		// Capture ANSI color values for the select list factory closure.
		let accent_ansi = self.fg_ansi(ThemeColor::Accent).to_owned();
		let muted_ansi = self.fg_ansi(ThemeColor::Muted).to_owned();
		let select_list_factory: Box<dyn Fn() -> SelectListTheme> = Box::new(move || {
			let make_closure = |ansi: &str| -> Box<dyn Fn(&str) -> String> {
				if ansi.is_empty() {
					Box::new(|s: &str| s.to_owned())
				} else {
					let ansi = ansi.to_owned();
					Box::new(move |s: &str| format!("{ansi}{s}\x1b[39m"))
				}
			};
			SelectListTheme {
				selected_prefix: make_closure(&accent_ansi),
				selected_text:   make_closure(&accent_ansi),
				description:     make_closure(&muted_ansi),
				scroll_info:     make_closure(&muted_ansi),
				no_match:        make_closure(&muted_ansi),
				symbols:         symbols.clone(),
			}
		});

		FilterableSelectTheme {
			tab_bar: self.tab_bar_theme(),
			select_list_factory,
			search_hint: self.fg_closure(ThemeColor::Dim),
			border: self.fg_closure(ThemeColor::BorderMuted),
		}
	}

	/// Build a `BorderColorFn` for the given color.
	pub fn border_color_fn(&self, color: ThemeColor) -> BorderColorFn {
		self.fg_closure(color)
	}

	/// Build a `HintStyleFn` using the `Dim` color.
	pub fn hint_style_fn(&self) -> HintStyleFn {
		self.fg_closure(ThemeColor::Dim)
	}

	/// Build `HighlightColors` for syntax highlighting.
	pub fn highlight_colors(&self) -> HighlightColors {
		HighlightColors {
			comment:     self.fg_ansi(ThemeColor::SyntaxComment).to_owned(),
			keyword:     self.fg_ansi(ThemeColor::SyntaxKeyword).to_owned(),
			function:    self.fg_ansi(ThemeColor::SyntaxFunction).to_owned(),
			variable:    self.fg_ansi(ThemeColor::SyntaxVariable).to_owned(),
			string:      self.fg_ansi(ThemeColor::SyntaxString).to_owned(),
			number:      self.fg_ansi(ThemeColor::SyntaxNumber).to_owned(),
			r#type:      self.fg_ansi(ThemeColor::SyntaxType).to_owned(),
			operator:    self.fg_ansi(ThemeColor::SyntaxOperator).to_owned(),
			punctuation: self.fg_ansi(ThemeColor::SyntaxPunctuation).to_owned(),
			inserted:    Some(self.fg_ansi(ThemeColor::ToolDiffAdded).to_owned()),
			deleted:     Some(self.fg_ansi(ThemeColor::ToolDiffRemoved).to_owned()),
		}
	}

	/// Create a `Box<dyn Fn(&str) -> String>` closure that applies fg color.
	/// This clones the ANSI string into the closure for independent ownership.
	fn fg_closure(&self, color: ThemeColor) -> Box<dyn Fn(&str) -> String> {
		let ansi = self.fg_ansi(color).to_owned();
		if ansi.is_empty() {
			Box::new(|s: &str| s.to_owned())
		} else {
			Box::new(move |s: &str| format!("{ansi}{s}\x1b[39m"))
		}
	}

	fn detect_color_mode() -> ColorMode {
		let colorterm = std::env::var("COLORTERM").unwrap_or_default();
		if colorterm == "truecolor" || colorterm == "24bit" {
			return ColorMode::TrueColor;
		}
		if std::env::var("WT_SESSION").is_ok() {
			return ColorMode::TrueColor;
		}
		let term = std::env::var("TERM").unwrap_or_default();
		if term.contains("256color") {
			return ColorMode::Color256;
		}
		// Most modern terminals support truecolor even without COLORTERM
		ColorMode::TrueColor
	}
}

/// Convert RGB to nearest 256-color index.
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
	// Use the 6x6x6 color cube (indices 16-231)
	let ri = (u16::from(r) * 5 / 255) as u8;
	let gi = (u16::from(g) * 5 / 255) as u8;
	let bi = (u16::from(b) * 5 / 255) as u8;
	16 + 36 * ri + 6 * gi + bi
}

/// Parse a hex color string like "#ff00aa" into (r, g, b).
fn parse_hex(hex: &str) -> (u8, u8, u8) {
	let hex = hex.trim_start_matches('#');
	if hex.len() < 6 {
		return (0, 0, 0);
	}
	let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
	let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
	let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
	(r, g, b)
}

/// Helper to create an RGB `ColorValue` from a hex string.
fn hex(s: &str) -> ColorValue {
	let (r, g, b) = parse_hex(s);
	ColorValue::Rgb(r, g, b)
}

/// Helper to create a 256-color index `ColorValue`.
const fn idx(n: u8) -> ColorValue {
	ColorValue::Index256(n)
}

/// Dark theme foreground colors (values from TypeScript dark.json).
fn dark_fg_colors() -> HashMap<ThemeColor, ColorValue> {
	use ThemeColor::*;
	HashMap::from([
		// Core UI
		(Accent, hex("#febc38")),
		(Border, hex("#178fb9")),
		(BorderAccent, hex("#0088fa")),
		(BorderMuted, hex("#3d424a")),
		(Success, hex("#89d281")),
		(Error, hex("#fc3a4b")),
		(Warning, hex("#e4c00f")),
		(Muted, hex("#777d88")),
		(Dim, hex("#5f6673")),
		(ThinkingText, hex("#777d88")),
		// Message text — Text, UserMessageText, CustomMessageText, ToolTitle
		// are "" in dark.json (terminal default), so they are intentionally
		// omitted here; fg() returns uncolored text when the key is absent.
		(CustomMessageLabel, hex("#b281d6")),
		(ToolOutput, hex("#777d88")),
		// Markdown
		(MdHeading, hex("#febc38")),
		(MdLink, hex("#0088fa")),
		(MdLinkUrl, hex("#5f6673")),
		(MdCode, hex("#e5c1ff")),
		(MdCodeBlock, hex("#9CDCFE")),
		(MdCodeBlockBorder, hex("#3d424a")),
		(MdQuote, hex("#777d88")),
		(MdQuoteBorder, hex("#3d424a")),
		(MdHr, hex("#3d424a")),
		(MdListBullet, hex("#febc38")),
		// Tool diffs
		(ToolDiffAdded, hex("#89d281")),
		(ToolDiffRemoved, hex("#fc3a4b")),
		(ToolDiffContext, hex("#777d88")),
		// Syntax highlighting
		(SyntaxComment, hex("#6A9955")),
		(SyntaxKeyword, hex("#569CD6")),
		(SyntaxFunction, hex("#DCDCAA")),
		(SyntaxVariable, hex("#9CDCFE")),
		(SyntaxString, hex("#CE9178")),
		(SyntaxNumber, hex("#B5CEA8")),
		(SyntaxType, hex("#4EC9B0")),
		(SyntaxOperator, hex("#D4D4D4")),
		(SyntaxPunctuation, hex("#D4D4D4")),
		// Thinking borders
		(ThinkingOff, hex("#3d424a")),
		(ThinkingMinimal, hex("#5f6673")),
		(ThinkingLow, hex("#178fb9")),
		(ThinkingMedium, hex("#0088fa")),
		(ThinkingHigh, hex("#b281d6")),
		(ThinkingXhigh, hex("#e5c1ff")),
		// Mode borders
		(BashMode, hex("#0088fa")),
		(PythonMode, hex("#e4c00f")),
		// Status line
		(StatusLineSep, idx(244)),
		(StatusLineModel, hex("#d787af")),
		(StatusLinePath, hex("#00afaf")),
		(StatusLineGitClean, hex("#5faf5f")),
		(StatusLineGitDirty, hex("#d7af5f")),
		(StatusLineContext, hex("#8787af")),
		(StatusLineSpend, hex("#5fafaf")),
		(StatusLineStaged, idx(70)),
		(StatusLineDirty, idx(178)),
		(StatusLineUntracked, idx(39)),
		(StatusLineOutput, idx(205)),
		(StatusLineCost, idx(205)),
		(StatusLineSubagents, hex("#febc38")),
	])
}

/// Dark theme background colors (values from TypeScript dark.json).
fn dark_bg_colors() -> HashMap<ThemeBg, ColorValue> {
	use ThemeBg::*;
	HashMap::from([
		(SelectedBg, hex("#31363f")),
		(UserMessageBg, hex("#221d1a")),
		(CustomMessageBg, hex("#2a2530")),
		(ToolPendingBg, hex("#1d2129")),
		(ToolSuccessBg, hex("#161a1f")),
		(ToolErrorBg, hex("#291d1d")),
		(StatusLineBg, hex("#121212")),
	])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_dark_theme_creates_without_panic() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		// Verify all colors are present
		let text = theme.fg(ThemeColor::Accent, "hello");
		assert!(text.contains("\x1b[38;2;"));
		assert!(text.contains("hello"));
		assert!(text.ends_with("\x1b[39m"));
	}

	#[test]
	fn test_fg_wraps_with_reset() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		let result = theme.fg(ThemeColor::Success, "ok");
		// Should be: \x1b[38;2;137;210;129m ok \x1b[39m
		assert!(result.starts_with("\x1b[38;2;"));
		assert!(result.contains("ok"));
		assert!(result.ends_with("\x1b[39m"));
	}

	#[test]
	fn test_bg_wraps_with_reset() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		let result = theme.bg(ThemeBg::UserMessageBg, "msg");
		assert!(result.starts_with("\x1b[48;2;"));
		assert!(result.contains("msg"));
		assert!(result.ends_with("\x1b[49m"));
	}

	#[test]
	fn test_bold_wraps() {
		let theme = Theme::dark();
		assert_eq!(theme.bold("x"), "\x1b[1mx\x1b[22m");
	}

	#[test]
	fn test_italic_wraps() {
		let theme = Theme::dark();
		assert_eq!(theme.italic("x"), "\x1b[3mx\x1b[23m");
	}

	#[test]
	fn test_dim_wraps() {
		let theme = Theme::dark();
		assert_eq!(theme.dim("x"), "\x1b[2mx\x1b[22m");
	}

	#[test]
	fn test_composability() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		// fg + bold should compose without clobbering
		let result = theme.fg(ThemeColor::Accent, &theme.bold("hello"));
		assert!(result.contains("\x1b[1m"));
		assert!(result.contains("hello"));
		assert!(result.contains("\x1b[22m")); // bold reset
		assert!(result.ends_with("\x1b[39m")); // fg reset
	}

	#[test]
	fn test_256_color_mode() {
		let theme = Theme::dark_with_mode(ColorMode::Color256);
		let result = theme.fg(ThemeColor::Accent, "x");
		assert!(result.starts_with("\x1b[38;5;"));
	}

	#[test]
	fn test_256_index_colors() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		// StatusLineSep uses idx(244) — should produce \x1b[38;5;244m
		let result = theme.fg(ThemeColor::StatusLineSep, "x");
		assert_eq!(result, "\x1b[38;5;244mx\x1b[39m");
	}

	#[test]
	fn test_rgb_to_256() {
		assert_eq!(rgb_to_256(255, 0, 0), 196); // bright red
		assert_eq!(rgb_to_256(0, 0, 0), 16); // black
		assert_eq!(rgb_to_256(255, 255, 255), 231); // white
	}

	#[test]
	fn test_parse_hex() {
		assert_eq!(parse_hex("#febc38"), (254, 188, 56));
		assert_eq!(parse_hex("ff0000"), (255, 0, 0));
		assert_eq!(parse_hex("#000000"), (0, 0, 0));
	}

	fn test_symbols() -> crate::symbols::SymbolTheme {
		crate::symbols::SymbolTheme {
			cursor:         ">",
			input_cursor:   "|",
			box_round:      crate::symbols::RoundedBoxSymbols {
				top_left:     "╭",
				top_right:    "╮",
				bottom_left:  "╰",
				bottom_right: "╯",
				horizontal:   "─",
				vertical:     "│",
			},
			box_sharp:      crate::symbols::BoxSymbols {
				top_left:     "┌",
				top_right:    "┐",
				bottom_left:  "└",
				bottom_right: "┘",
				horizontal:   "─",
				vertical:     "│",
				tee_down:     "┬",
				tee_up:       "┴",
				tee_left:     "┤",
				tee_right:    "├",
				cross:        "┼",
			},
			table:          crate::symbols::BoxSymbols {
				top_left:     "┌",
				top_right:    "┐",
				bottom_left:  "└",
				bottom_right: "┘",
				horizontal:   "─",
				vertical:     "│",
				tee_down:     "┬",
				tee_up:       "┴",
				tee_left:     "┤",
				tee_right:    "├",
				cross:        "┼",
			},
			tree:           crate::symbols::TreeSymbols {
				branch:   "├─",
				last:     "╰─",
				vertical: "│",
			},
			quote_border:   "│",
			hr_char:        "─",
			spinner_frames: &["⠋"],
		}
	}

	#[test]
	fn test_markdown_theme_styling() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		let symbols = test_symbols();
		let md_theme = theme.markdown_theme(symbols);
		let heading = (md_theme.heading)("Title");
		assert!(heading.contains("Title"));
		assert!(heading.contains("\x1b[38;2;")); // colored
	}

	#[test]
	fn test_select_list_theme() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		let symbols = test_symbols();
		let sl_theme = theme.select_list_theme(symbols);
		let selected = (sl_theme.selected_text)("item");
		assert!(selected.contains("item"));
	}

	#[test]
	fn test_highlight_colors() {
		let theme = Theme::dark_with_mode(ColorMode::TrueColor);
		let colors = theme.highlight_colors();
		assert!(colors.keyword.contains("\x1b[38;2;"));
		assert!(colors.string.contains("\x1b[38;2;"));
	}
}
