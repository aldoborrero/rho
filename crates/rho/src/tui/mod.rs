pub mod autocomplete;
pub mod chat;
pub mod status;
pub mod tool_renderers;
pub mod welcome;

use std::{path::PathBuf, rc::Rc};

use autocomplete::RhoAutocompleteProvider;
use chat::ChatComponent;
use rho_tui::{
	capabilities::{TerminalInfo, detect_terminal_id, get_terminal_info},
	component::{Component, Focusable, InputResult},
	components::{editor::Editor, spacer::Spacer},
	is_key_release,
	symbols::{BoxSymbols, RoundedBoxSymbols, SymbolTheme, TreeSymbols},
	terminal::{CrosstermTerminal, Terminal},
	theme::{Theme, ThemeColor},
	tui::Tui,
};
use status::StatusLineComponent;
use welcome::WelcomeComponent;

// ── Default symbol theme ────────────────────────────────────────────────

/// Default symbol theme for the TUI (rounded Unicode box-drawing characters).
const fn default_symbols() -> SymbolTheme {
	SymbolTheme {
		cursor:         ">",
		input_cursor:   "|",
		box_round:      RoundedBoxSymbols {
			top_left:     "\u{256d}",
			top_right:    "\u{256e}",
			bottom_left:  "\u{2570}",
			bottom_right: "\u{256f}",
			horizontal:   "\u{2500}",
			vertical:     "\u{2502}",
		},
		box_sharp:      BoxSymbols {
			top_left:     "\u{250c}",
			top_right:    "\u{2510}",
			bottom_left:  "\u{2514}",
			bottom_right: "\u{2518}",
			horizontal:   "\u{2500}",
			vertical:     "\u{2502}",
			tee_down:     "\u{252c}",
			tee_up:       "\u{2534}",
			tee_left:     "\u{2524}",
			tee_right:    "\u{251c}",
			cross:        "\u{253c}",
		},
		table:          BoxSymbols {
			top_left:     "\u{250c}",
			top_right:    "\u{2510}",
			bottom_left:  "\u{2514}",
			bottom_right: "\u{2518}",
			horizontal:   "\u{2500}",
			vertical:     "\u{2502}",
			tee_down:     "\u{252c}",
			tee_up:       "\u{2534}",
			tee_left:     "\u{2524}",
			tee_right:    "\u{251c}",
			cross:        "\u{253c}",
		},
		tree:           TreeSymbols {
			branch:   "\u{251c}\u{2500}",
			last:     "\u{2570}\u{2500}",
			vertical: "\u{2502}",
		},
		quote_border:   "\u{2502}",
		hr_char:        "\u{2500}",
		spinner_frames: &[
			"\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
			"\u{2827}", "\u{2807}", "\u{280f}",
		],
	}
}

// ── App ─────────────────────────────────────────────────────────────────

/// Top-level TUI application that owns the differential renderer and all
/// visible components (chat history, status bar, text editor).
///
/// App owns components directly as named struct fields — no `Rc<RefCell<T>>`.
/// Tui is a pure differential renderer that receives pre-rendered lines.
pub struct App {
	pub tui:     Tui,
	#[allow(dead_code, reason = "holds Rc ownership — theme closures captured by editor")]
	pub theme:   Rc<Theme>,
	pub chat:    ChatComponent,
	pub status:  StatusLineComponent,
	pub editor:  Editor,
	pub welcome: WelcomeComponent,
}

impl App {
	/// Create a new `App` with the given model name displayed in the status bar.
	///
	/// Terminal capabilities are auto-detected from environment variables.
	/// The editor is focused by default so it is ready for user input.
	pub fn new(model: &str) -> Self {
		let terminal_info = Self::detect_terminal_info();
		Self::with_terminal_info(model, terminal_info)
	}

	/// Create a new `App` with an explicit `TerminalInfo` (useful for testing).
	pub fn with_terminal_info(model: &str, terminal_info: TerminalInfo) -> Self {
		let tui = Tui::new(terminal_info);
		let symbols = default_symbols();
		let theme = Rc::new(Theme::dark());

		// Create themed components.
		let chat = ChatComponent::new(Rc::clone(&theme), symbols.clone());
		let status = StatusLineComponent::new(Rc::clone(&theme), model);
		let welcome = WelcomeComponent::new(
			Rc::clone(&theme),
			env!("CARGO_PKG_VERSION").to_owned(),
			model.to_owned(),
			vec![],
		);

		// Create themed editor.
		let select_list_theme_fn = {
			let theme = Rc::clone(&theme);
			let symbols = symbols.clone();
			Box::new(move || theme.select_list_theme(symbols.clone()))
		};
		let mut editor = Editor::new(
			theme.border_color_fn(ThemeColor::BorderMuted),
			select_list_theme_fn,
			symbols,
			Some(2),
			Some(theme.hint_style_fn()),
		);
		editor.set_focused(true);

		// Wire up autocomplete for slash commands and file paths.
		let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
		let provider = RhoAutocompleteProvider::new(cwd);
		editor.set_autocomplete_provider(Box::new(provider));

		Self { tui, theme, chat, status, editor, welcome }
	}

	/// Render all components and write to the terminal via Tui's
	/// differential renderer.
	pub fn render_to_tui(&mut self, terminal: &mut dyn Terminal) -> std::io::Result<()> {
		let width = terminal.columns();
		let mut lines = Vec::new();
		lines.extend(Spacer::new(1).render(width));
		lines.extend(self.welcome.render(width));
		lines.extend(Spacer::new(1).render(width));
		lines.extend(self.chat.render(width));
		lines.extend(Spacer::new(1).render(width));
		lines.extend(self.editor.render(width));
		lines.extend(Spacer::new(1).render(width));
		self.tui.render_lines(&lines, terminal)
	}

	/// Handle raw terminal input. Routes through Tui's input listeners,
	/// filters key releases, and forwards to the editor.
	pub fn handle_input(&mut self, data: &str) -> InputResult {
		// Run input listeners (e.g., overlay dismiss handlers)
		let Some(current) = self.tui.process_input_listeners(data) else {
			return InputResult::Consumed;
		};

		// Filter key release events unless editor opts in
		if !self.editor.wants_key_release() && is_key_release(&current) {
			return InputResult::Consumed;
		}

		// Forward to editor (always focused)
		let result = self.editor.handle_input(&current);
		self.tui.request_render();
		result
	}

	/// Produce an `EditorTopBorder` from the status line and apply it to the
	/// editor.
	pub fn update_status_border(&mut self, width: u16) {
		let border = self.status.get_top_border(width);
		self.editor.set_top_border(Some(border));
	}

	/// Update the editor border color based on the current input prefix.
	///
	/// Uses `BashMode` (blue) when the text starts with `!`, otherwise
	/// falls back to the default `BorderMuted`.
	pub fn sync_editor_border_color(&mut self) {
		let text = self.editor.get_text();
		let trimmed = text.trim_start();
		let color = if trimmed.starts_with('!') {
			ThemeColor::BashMode
		} else {
			ThemeColor::BorderMuted
		};
		self.editor.border_color = self.theme.border_color_fn(color);
	}

	/// Detect terminal capabilities from the environment.
	fn detect_terminal_info() -> TerminalInfo {
		get_terminal_info(detect_terminal_id())
	}
}

// ── Terminal helpers ────────────────────────────────────────────────────

/// Start a crossterm-based terminal in raw mode with bracketed paste
/// and (optionally) the Kitty keyboard protocol.
///
/// # Errors
///
/// Returns an error if raw mode cannot be enabled.
pub fn start_terminal() -> anyhow::Result<CrosstermTerminal> {
	let mut terminal = CrosstermTerminal::new();
	terminal.start()?;
	Ok(terminal)
}
