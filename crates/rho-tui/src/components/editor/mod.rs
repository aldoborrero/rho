//! Multi-line editor component with autocomplete, history, kill ring, and word
//! wrapping.
//!
//! This is a faithful port of `packages/tui/src/components/editor.ts` (2,235
//! lines). It provides a full-featured terminal text editor with:
//! - Grapheme-aware cursor movement and editing
//! - Word-boundary-aware text wrapping
//! - Emacs-style kill ring (Ctrl+K/U/W, Ctrl+Y, Alt+Y)
//! - Undo stack with suspend/resume for compound operations
//! - Prompt history navigation (Up/Down)
//! - Autocomplete integration with debounced updates
//! - Character jump mode (Ctrl+], Ctrl+Alt+])
//! - Bracketed paste with large paste compression
//! - Sticky column for vertical cursor movement

pub mod editing;
pub mod history;
pub mod layout;
pub mod motion;
pub mod state;

use unicode_segmentation::UnicodeSegmentation;

use self::{
	editing::set_text_internal,
	history::History,
	layout::layout_text,
	motion::{
		is_on_first_visual_line, is_on_last_visual_line, jump_to_char, move_cursor, move_to_line_end,
		move_to_line_start, move_word_backwards, move_word_forwards, set_cursor_col,
	},
	state::{EditorState, LayoutLine},
};
use super::{
	select_list::{SelectItem, SelectList, SelectListTheme},
	text::make_padding,
};
use crate::{
	component::{CURSOR_MARKER, Component, Focusable, InputResult},
	keybindings::{EditorAction, get_editor_keybindings},
	kill_ring::KillRing,
	symbols::{RoundedBoxSymbols, SymbolTheme},
};

/// Style function type for border coloring.
pub type BorderColorFn = Box<dyn Fn(&str) -> String>;

/// Style function for inline hint/ghost text.
pub type HintStyleFn = Box<dyn Fn(&str) -> String>;

/// Callback type for editor events with text argument.
pub type TextCallback = Option<Box<dyn FnMut(&str)>>;

/// Theme for the editor component.
pub struct EditorTheme {
	pub border_color:     BorderColorFn,
	pub select_list:      SelectListTheme,
	pub symbols:          SymbolTheme,
	pub editor_padding_x: Option<usize>,
	pub hint_style:       Option<HintStyleFn>,
}

/// Custom content for the top border (e.g., status line).
pub struct EditorTopBorder {
	/// The status content (already styled).
	pub content: String,
	/// Visible width of the content.
	pub width:   usize,
}

/// Autocomplete suggestion result.
pub struct AutocompleteSuggestions {
	pub items:  Vec<SelectItem>,
	pub prefix: String,
}

/// Autocomplete provider trait.
pub trait AutocompleteProvider {
	/// Get suggestions for the current editor state.
	fn get_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<AutocompleteSuggestions>;

	/// Apply a selected completion.
	fn apply_completion(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
		item: &SelectItem,
		prefix: &str,
	) -> CompletionResult;

	/// Get inline hint text to show as ghost text after cursor.
	fn get_inline_hint(
		&self,
		_lines: &[String],
		_cursor_line: usize,
		_cursor_col: usize,
	) -> Option<String> {
		None
	}

	/// Whether Tab should trigger file completion at this position.
	fn should_trigger_file_completion(
		&self,
		_lines: &[String],
		_cursor_line: usize,
		_cursor_col: usize,
	) -> bool {
		true
	}

	/// Get force-file suggestions (for Tab completion).
	fn get_force_file_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<AutocompleteSuggestions> {
		self.get_suggestions(lines, cursor_line, cursor_col)
	}
}

/// Result of applying a completion.
pub struct CompletionResult {
	pub lines:       Vec<String>,
	pub cursor_line: usize,
	pub cursor_col:  usize,
}

/// Tracked action for kill-ring accumulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
	Kill,
	Yank,
}

/// Autocomplete state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutocompleteState {
	Regular,
	Force,
}

/// Jump mode direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpMode {
	Forward,
	Backward,
}

/// Kitty CSI-u modifier bits.
const KITTY_MOD_SHIFT: u32 = 1;
const KITTY_MOD_ALT: u32 = 2;
const KITTY_MOD_CTRL: u32 = 4;

/// Multi-line editor component.
#[allow(clippy::struct_excessive_bools, reason = "editor has many boolean flags by design")]
pub struct Editor {
	state:   EditorState,
	focused: bool,

	theme_select_list_fn: Box<dyn Fn() -> SelectListTheme>,
	symbols:              SymbolTheme,
	theme_padding_x:      Option<usize>,
	hint_style:           Option<HintStyleFn>,

	use_terminal_cursor:       bool,
	/// When set, replaces the normal cursor glyph at end-of-text.
	pub cursor_override:       Option<String>,
	/// Display width of the cursor override (ANSI-safe width).
	pub cursor_override_width: Option<usize>,

	last_layout_width:  usize,
	padding_x_override: Option<usize>,
	max_height:         Option<usize>,
	scroll_offset:      usize,

	// Kill ring
	kill_ring:   KillRing,
	last_action: Option<LastAction>,

	// Character jump mode
	jump_mode: Option<JumpMode>,

	// Sticky column for vertical cursor movement
	preferred_visual_col: Option<usize>,

	// Dynamic border color
	pub border_color: BorderColorFn,

	// Autocomplete
	autocomplete_provider:    Option<Box<dyn AutocompleteProvider>>,
	autocomplete_list:        Option<SelectList>,
	autocomplete_state:       Option<AutocompleteState>,
	autocomplete_prefix:      String,
	autocomplete_request_id:  u64,
	autocomplete_max_visible: usize,

	// Paste tracking
	pastes:        Vec<(u64, String)>,
	paste_counter: u64,

	// Bracketed paste buffering
	paste_buffer: String,
	is_in_paste:  bool,

	// Prompt history
	history: History,

	// Undo
	undo_stack:   Vec<EditorState>,
	suspend_undo: bool,

	// Custom top border
	top_border_content: Option<EditorTopBorder>,

	// Callbacks
	pub on_alt_enter:           TextCallback,
	pub on_change:              TextCallback,
	pub on_autocomplete_update: Option<Box<dyn FnMut()>>,
	pub on_autocomplete_cancel: Option<Box<dyn FnMut()>>,
	pub disable_submit:         bool,
}

impl Editor {
	/// Create a new editor. The `select_list_factory` is called when creating
	/// autocomplete dropdown lists (since `SelectListTheme` contains boxed
	/// closures and cannot be stored directly).
	pub fn new(
		border_color: BorderColorFn,
		select_list_factory: Box<dyn Fn() -> SelectListTheme>,
		symbols: SymbolTheme,
		theme_padding_x: Option<usize>,
		hint_style: Option<HintStyleFn>,
	) -> Self {
		Self {
			state: EditorState::default(),
			focused: false,
			theme_select_list_fn: select_list_factory,
			symbols,
			theme_padding_x,
			hint_style,
			use_terminal_cursor: false,
			cursor_override: None,
			cursor_override_width: None,
			last_layout_width: 80,
			padding_x_override: None,
			max_height: None,
			scroll_offset: 0,
			kill_ring: KillRing::new(),
			last_action: None,
			jump_mode: None,
			preferred_visual_col: None,
			border_color,
			autocomplete_provider: None,
			autocomplete_list: None,
			autocomplete_state: None,
			autocomplete_prefix: String::new(),
			autocomplete_request_id: 0,
			autocomplete_max_visible: 5,
			pastes: Vec::new(),
			paste_counter: 0,
			paste_buffer: String::new(),
			is_in_paste: false,
			history: History::new(),
			undo_stack: Vec::new(),
			suspend_undo: false,
			top_border_content: None,
			on_alt_enter: None,
			on_change: None,
			on_autocomplete_update: None,
			on_autocomplete_cancel: None,
			disable_submit: false,
		}
	}

	// ── Public API ──────────────────────────────────────────────────

	pub fn set_autocomplete_provider(&mut self, provider: Box<dyn AutocompleteProvider>) {
		self.autocomplete_provider = Some(provider);
	}

	pub fn set_top_border(&mut self, content: Option<EditorTopBorder>) {
		self.top_border_content = content;
	}

	pub const fn set_use_terminal_cursor(&mut self, use_terminal_cursor: bool) {
		self.use_terminal_cursor = use_terminal_cursor;
	}

	pub const fn get_use_terminal_cursor(&self) -> bool {
		self.use_terminal_cursor
	}

	pub const fn set_max_height(&mut self, max_height: Option<usize>) {
		self.max_height = max_height;
		self.scroll_offset = 0;
	}

	pub const fn set_padding_x(&mut self, padding_x: usize) {
		self.padding_x_override = Some(padding_x);
	}

	pub const fn get_autocomplete_max_visible(&self) -> usize {
		self.autocomplete_max_visible
	}

	pub fn set_autocomplete_max_visible(&mut self, max_visible: usize) {
		self.autocomplete_max_visible = max_visible.clamp(3, 20);
	}

	pub fn set_history_entries(&mut self, entries: Vec<String>) {
		self.history.load(entries);
	}

	pub fn add_to_history(&mut self, text: &str) {
		self.history.add(text);
	}

	pub fn get_text(&self) -> String {
		self.state.text()
	}

	/// Get text with paste markers expanded to their actual content.
	pub fn get_expanded_text(&self) -> String {
		let mut result = self.state.text();
		for (paste_id, paste_content) in &self.pastes {
			// Replace markers like "[paste #1 +123 lines]" or "[paste #1 1234 chars]"
			let pattern = format!("[paste #{paste_id}");
			while let Some(start) = result.find(&pattern) {
				if let Some(end) = result[start..].find(']') {
					result.replace_range(start..=(start + end), paste_content);
				} else {
					break;
				}
			}
		}
		result
	}

	pub fn get_lines(&self) -> Vec<String> {
		self.state.lines.clone()
	}

	pub const fn get_cursor(&self) -> (usize, usize) {
		(self.state.cursor_line, self.state.cursor_col)
	}

	pub fn set_text(&mut self, text: &str) {
		self.history.reset();
		self.reset_kill_sequence();
		self.preferred_visual_col = None;
		self.undo_stack.clear();
		set_text_internal(&mut self.state, text);
		self.notify_change();
	}

	/// Insert text at the current cursor position.
	pub fn insert_text(&mut self, text: &str) {
		self.history.reset();
		self.reset_kill_sequence();
		self.record_undo_state();
		editing::insert_text_at_cursor(&mut self.state, text);
		self.notify_change();
	}

	pub const fn is_showing_autocomplete(&self) -> bool {
		self.autocomplete_state.is_some()
	}

	// ── Internal helpers ────────────────────────────────────────────

	fn get_editor_padding_x(&self) -> usize {
		self
			.padding_x_override
			.or(self.theme_padding_x)
			.unwrap_or(2)
	}

	const fn content_width(width: usize, padding_x: usize) -> usize {
		width.saturating_sub(2 * (padding_x + 1))
	}

	fn layout_width(width: usize, padding_x: usize) -> usize {
		let content_width = Self::content_width(width, padding_x);
		let w = if padding_x == 0 {
			content_width.saturating_sub(1)
		} else {
			content_width
		};
		w.max(1)
	}

	fn get_visible_content_height(&self, content_lines: usize) -> usize {
		match self.max_height {
			None => content_lines,
			Some(max_h) => max_h.saturating_sub(2).max(1),
		}
	}

	fn update_scroll_offset(
		&mut self,
		layout_width: usize,
		layout_lines: &[LayoutLine],
		visible_height: usize,
	) {
		if layout_lines.len() <= visible_height {
			self.scroll_offset = 0;
			return;
		}

		let visual_lines = layout::build_visual_line_map(&self.state, layout_width);
		let cursor_line = layout::find_current_visual_line(&self.state, &visual_lines);
		if cursor_line < self.scroll_offset {
			self.scroll_offset = cursor_line;
		} else if cursor_line >= self.scroll_offset + visible_height {
			self.scroll_offset = cursor_line - visible_height + 1;
		}

		let max_offset = layout_lines.len().saturating_sub(visible_height);
		self.scroll_offset = self.scroll_offset.min(max_offset);
	}

	fn notify_change(&mut self) {
		if let Some(ref mut cb) = self.on_change {
			cb(&self.state.text());
		}
	}

	const fn reset_kill_sequence(&mut self) {
		self.last_action = None;
	}

	fn record_undo_state(&mut self) {
		if !self.suspend_undo {
			self.undo_stack.push(self.state.clone());
		}
	}

	fn apply_undo(&mut self) {
		let Some(snapshot) = self.undo_stack.pop() else {
			return;
		};
		self.history.reset();
		self.reset_kill_sequence();
		self.preferred_visual_col = None;
		self.state = snapshot;
		self.notify_change();

		if self.autocomplete_state.is_some() {
			self.update_autocomplete();
		} else {
			self.try_retrigger_autocomplete();
		}
	}

	fn record_kill(&mut self, text: &str, direction_backward: bool) {
		if text.is_empty() {
			return;
		}
		let accumulate = self.last_action == Some(LastAction::Kill);
		self.kill_ring.push(text, direction_backward, accumulate);
		self.last_action = Some(LastAction::Kill);
	}

	fn is_at_start_of_message(&self) -> bool {
		let before = self.state.text_before_cursor();
		let trimmed = before.trim();
		trimmed.is_empty() || trimmed == "/"
	}

	// ── History navigation ──────────────────────────────────────────

	fn navigate_history(&mut self, direction: i32) {
		self.reset_kill_sequence();
		match self.history.navigate(direction) {
			Ok(Some(text)) => {
				self.undo_stack.clear();
				set_text_internal(&mut self.state, &text);
				self.notify_change();
			},
			Ok(None) => {
				// Returned to current state — clear editor
				self.undo_stack.clear();
				set_text_internal(&mut self.state, "");
				self.notify_change();
			},
			Err(()) => {}, // No-op
		}
	}

	// ── Kill ring operations ────────────────────────────────────────

	fn yank_from_kill_ring(&mut self) {
		let Some(text) = self.kill_ring.peek().map(str::to_owned) else {
			return;
		};
		editing::insert_text_at_cursor(&mut self.state, &text);
		self.last_action = Some(LastAction::Yank);
		self.notify_change();
	}

	fn yank_pop(&mut self) {
		if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
			return;
		}

		self.history.reset();
		self.record_undo_state();

		let yanked = self.kill_ring.peek().map(str::to_owned);
		if let Some(ref yanked) = yanked
			&& !editing::delete_yanked_text(&mut self.state, yanked)
		{
			return;
		}
		self.kill_ring.rotate();
		if let Some(new_text) = self.kill_ring.peek().map(str::to_owned) {
			editing::insert_text_at_cursor(&mut self.state, &new_text);
		}

		self.last_action = Some(LastAction::Yank);
		self.notify_change();
	}

	// ── Autocomplete ────────────────────────────────────────────────

	fn try_trigger_autocomplete(&mut self) {
		let Some(ref mut provider) = self.autocomplete_provider else {
			return;
		};

		self.autocomplete_request_id += 1;

		let suggestions =
			provider.get_suggestions(&self.state.lines, self.state.cursor_line, self.state.cursor_col);

		if let Some(suggestions) = suggestions
			&& !suggestions.items.is_empty()
		{
			self.autocomplete_prefix = suggestions.prefix;
			self.autocomplete_list = Some(SelectList::new(
				suggestions.items,
				self.autocomplete_max_visible,
				(self.theme_select_list_fn)(),
			));
			self.autocomplete_state = Some(AutocompleteState::Regular);
			if let Some(ref mut cb) = self.on_autocomplete_update {
				cb();
			}
			return;
		}

		self.cancel_autocomplete(false);
		if let Some(ref mut cb) = self.on_autocomplete_update {
			cb();
		}
	}

	fn force_file_autocomplete(&mut self, explicit_tab: bool) {
		let Some(ref mut provider) = self.autocomplete_provider else {
			return;
		};

		self.autocomplete_request_id += 1;

		let suggestions = provider.get_force_file_suggestions(
			&self.state.lines,
			self.state.cursor_line,
			self.state.cursor_col,
		);

		if let Some(suggestions) = suggestions
			&& !suggestions.items.is_empty()
		{
			// Single suggestion + explicit Tab → apply immediately
			if explicit_tab && suggestions.items.len() == 1 {
				let item = suggestions.items[0].clone();
				let result = provider.apply_completion(
					&self.state.lines,
					self.state.cursor_line,
					self.state.cursor_col,
					&item,
					&suggestions.prefix,
				);
				self.state.lines = result.lines;
				self.state.cursor_line = result.cursor_line;
				set_cursor_col(&mut self.state, result.cursor_col, &mut self.preferred_visual_col);
				self.notify_change();
				return;
			}

			self.autocomplete_prefix = suggestions.prefix;
			self.autocomplete_list = Some(SelectList::new(
				suggestions.items,
				self.autocomplete_max_visible,
				(self.theme_select_list_fn)(),
			));
			self.autocomplete_state = Some(AutocompleteState::Force);
			if let Some(ref mut cb) = self.on_autocomplete_update {
				cb();
			}
			return;
		}

		self.cancel_autocomplete(false);
		if let Some(ref mut cb) = self.on_autocomplete_update {
			cb();
		}
	}

	fn update_autocomplete(&mut self) {
		if self.autocomplete_state.is_none() || self.autocomplete_provider.is_none() {
			return;
		}

		if self.autocomplete_state == Some(AutocompleteState::Force) {
			self.force_file_autocomplete(false);
			return;
		}

		let Some(ref mut provider) = self.autocomplete_provider else {
			return;
		};

		self.autocomplete_request_id += 1;

		let suggestions =
			provider.get_suggestions(&self.state.lines, self.state.cursor_line, self.state.cursor_col);

		if let Some(suggestions) = suggestions
			&& !suggestions.items.is_empty()
		{
			self.autocomplete_prefix = suggestions.prefix;
			self.autocomplete_list = Some(SelectList::new(
				suggestions.items,
				self.autocomplete_max_visible,
				(self.theme_select_list_fn)(),
			));
			if let Some(ref mut cb) = self.on_autocomplete_update {
				cb();
			}
			return;
		}

		self.cancel_autocomplete(false);
		if let Some(ref mut cb) = self.on_autocomplete_update {
			cb();
		}
	}

	fn cancel_autocomplete(&mut self, notify: bool) {
		let was_active = self.autocomplete_state.is_some();
		self.autocomplete_request_id += 1;
		self.autocomplete_state = None;
		self.autocomplete_list = None;
		self.autocomplete_prefix.clear();
		if notify
			&& was_active
			&& let Some(ref mut cb) = self.on_autocomplete_cancel
		{
			cb();
		}
	}

	fn handle_tab_completion(&mut self) {
		if self.autocomplete_provider.is_none() {
			return;
		}

		let before = self.state.text_before_cursor().to_owned();
		let trimmed = before.trim_start();

		if trimmed.starts_with('/') && !trimmed.contains(' ') {
			self.try_trigger_autocomplete();
		} else {
			self.force_file_autocomplete(true);
		}
	}

	fn try_retrigger_autocomplete(&mut self) {
		let before = self.state.text_before_cursor().to_owned();
		let trimmed = before.trim_start();
		if trimmed.starts_with('/')
			|| (before.contains('@')
				&& before.rfind('@').is_some_and(|pos| {
					// Check that @ is preceded by whitespace or is at start
					pos == 0
						|| before
							.as_bytes()
							.get(pos - 1)
							.is_some_and(|&b| b == b' ' || b == b'\t')
				})) {
			self.try_trigger_autocomplete();
		}
	}

	fn get_inline_hint(&self) -> Option<String> {
		// Check selected autocomplete item for a hint
		if self.autocomplete_state.is_some()
			&& let Some(ref list) = self.autocomplete_list
			&& let Some(item) = list.selected_item()
		{
			return item.hint.clone();
		}

		// Fall back to provider's inline hint
		if let Some(ref provider) = self.autocomplete_provider {
			return provider.get_inline_hint(
				&self.state.lines,
				self.state.cursor_line,
				self.state.cursor_col,
			);
		}

		None
	}

	// ── Paste handling ──────────────────────────────────────────────

	fn handle_paste(&mut self, pasted_text: &str) {
		self.history.reset();
		self.reset_kill_sequence();
		self.record_undo_state();

		// Clean the pasted text
		let clean = pasted_text.replace("\r\n", "\n").replace('\r', "\n");
		// Convert tabs to 4 spaces
		let tab_expanded = clean.replace('\t', "    ");
		// Filter non-printable characters except newlines
		let filtered: String = tab_expanded
			.chars()
			.filter(|&c| c == '\n' || c >= ' ')
			.collect();

		let mut text = filtered;

		// If pasting a file path, prepend space if preceded by a word char
		if (text.starts_with('/') || text.starts_with('~') || text.starts_with('.'))
			&& self.state.cursor_col > 0
		{
			let line = self.state.current_line();
			if let Some(ch) = line[..self.state.cursor_col].chars().next_back()
				&& (ch.is_alphanumeric() || ch == '_')
			{
				text = format!(" {text}");
			}
		}

		let line_count = text.matches('\n').count() + 1;
		let total_chars = text.len();

		// Large paste → compress to marker
		if line_count > 10 || total_chars > 1000 {
			self.paste_counter += 1;
			let paste_id = self.paste_counter;

			let marker = if line_count > 10 {
				format!("[paste #{paste_id} +{line_count} lines]")
			} else {
				format!("[paste #{paste_id} {total_chars} chars]")
			};
			self.pastes.push((paste_id, text));
			editing::insert_text_at_cursor(&mut self.state, &marker);
			self.notify_change();
			return;
		}

		if line_count == 1 {
			// Single line — insert character by character for autocomplete triggers
			for grapheme in text.graphemes(true) {
				self.insert_character_internal(grapheme);
			}
		} else {
			editing::insert_text_at_cursor(&mut self.state, &text);
			self.notify_change();
		}
	}

	// ── Character insertion (with autocomplete trigger logic) ───────

	fn insert_character_internal(&mut self, ch: &str) {
		self.history.reset();
		self.reset_kill_sequence();
		self.record_undo_state();

		editing::insert_character(&mut self.state, ch);
		self.notify_change();

		// Autocomplete trigger logic
		if self.autocomplete_state.is_none() {
			if ch == "/" && self.is_at_start_of_message() {
				self.try_trigger_autocomplete();
			} else if ch == "@" {
				let before = self.state.text_before_cursor();
				let len = before.len();
				if len == 1
					|| before
						.as_bytes()
						.get(len - 2)
						.is_some_and(|&b| b == b' ' || b == b'\t')
				{
					self.try_trigger_autocomplete();
				}
			} else if ch.len() == 1 && ch.as_bytes()[0].is_ascii_alphanumeric()
				|| ch == "."
				|| ch == "-"
				|| ch == "_"
				|| ch == "/"
			{
				let before = self.state.text_before_cursor().to_owned();
				let trimmed = before.trim_start();
				if trimmed.starts_with('/') {
					self.try_trigger_autocomplete();
				} else {
					// Check for @ file reference context
					let has_at_ref = before.rfind('@').is_some_and(|pos| {
						(pos == 0
							|| before
								.as_bytes()
								.get(pos - 1)
								.is_some_and(|&b| b == b' ' || b == b'\t'))
							&& !before[pos..].contains(char::is_whitespace)
					});
					if has_at_ref {
						self.try_trigger_autocomplete();
					}
				}
			}
		} else {
			self.update_autocomplete();
		}
	}

	fn submit_value(&mut self) -> Option<String> {
		self.reset_kill_sequence();

		let mut result = self.state.lines.join("\n");
		let trimmed = result.trim().to_owned();

		// Expand paste markers
		for (paste_id, paste_content) in &self.pastes {
			let pattern = format!("[paste #{paste_id}");
			while let Some(start) = result.find(&pattern) {
				if let Some(end) = result[start..].find(']') {
					result.replace_range(start..=(start + end), paste_content);
				} else {
					break;
				}
			}
		}

		// Reset state
		self.state = EditorState::default();
		self.pastes.clear();
		self.paste_counter = 0;
		self.history.reset();
		self.scroll_offset = 0;
		self.undo_stack.clear();

		if let Some(ref mut cb) = self.on_change {
			cb("");
		}

		if trimmed.is_empty() { None } else { Some(result) }
	}

	// ── Kitty CSI-u decoding ────────────────────────────────────────

	/// Decode a Kitty CSI-u printable character sequence.
	fn decode_kitty_printable(data: &str) -> Option<String> {
		// CSI <codepoint>[:<shifted>[:<base>]][;<mod>[:<event>]]u
		let bytes = data.as_bytes();
		if bytes.len() < 4 || bytes[0] != 0x1b || bytes[1] != b'[' || *bytes.last()? != b'u' {
			return None;
		}

		let inner = &data[2..data.len() - 1]; // between [ and u
		let parts: Vec<&str> = inner.split(';').collect();
		if parts.is_empty() {
			return None;
		}

		// Parse codepoint part: <codepoint>[:<shifted>[:<base>]]
		let cp_parts: Vec<&str> = parts[0].split(':').collect();
		let codepoint: u32 = cp_parts[0].parse().ok()?;

		let shifted_key: Option<u32> = cp_parts
			.get(1)
			.filter(|s| !s.is_empty())
			.and_then(|s| s.parse().ok());

		// Parse modifier
		let mod_part: Vec<&str> = parts
			.get(1)
			.map_or_else(|| vec!["1"], |s| s.split(':').collect());
		let mod_value: u32 = mod_part[0].parse().unwrap_or(1);
		let modifier = mod_value.saturating_sub(1);

		// Ignore Alt/Ctrl shortcuts
		if modifier & (KITTY_MOD_ALT | KITTY_MOD_CTRL) != 0 {
			return None;
		}

		// Check for text field
		// Format: CSI codepoint[;modifier[;text_field]]u
		// The text field contains colon-separated codepoints
		let text_field: Option<&str> = parts.get(2).copied().or_else(|| {
			parts.get(1).and_then(|p| {
				if p.split(':').count() > 1 {
					Some(*p)
				} else {
					None
				}
			})
		});
		if let Some(text_field) = text_field
			&& !text_field.is_empty()
		{
			let codepoints: Vec<u32> = text_field
				.split(':')
				.filter(|s| !s.is_empty())
				.filter_map(|s| s.parse().ok())
				.filter(|&v| v >= 32)
				.collect();
			if !codepoints.is_empty() {
				let result: String = codepoints
					.iter()
					.filter_map(|&cp| char::from_u32(cp))
					.collect();
				if !result.is_empty() {
					return Some(result);
				}
			}
		}

		// Prefer shifted keycode when Shift is held
		let effective = if modifier & KITTY_MOD_SHIFT != 0 {
			shifted_key.unwrap_or(codepoint)
		} else {
			codepoint
		};

		// Reject private use area
		if (0xe000..=0xf8ff).contains(&effective) {
			return None;
		}
		// Reject control characters
		if effective < 32 {
			return None;
		}

		char::from_u32(effective).map(|c| c.to_string())
	}
}

// ── Component trait implementation ──────────────────────────────

impl Component for Editor {
	#[allow(
		clippy::too_many_lines,
		reason = "render method builds complete box-bordered editor output"
	)]
	fn render(&mut self, width: u16) -> Vec<String> {
		let width = width as usize;
		let padding_x = self.get_editor_padding_x();
		let content_area_width = Self::content_width(width, padding_x);
		let layout_width = Self::layout_width(width, padding_x);

		// Update layout width and scroll offset (previously in render_mut).
		self.last_layout_width = layout_width;
		{
			let layout_lines = layout_text(&self.state, layout_width);
			let visible_height = self.get_visible_content_height(layout_lines.len());
			self.update_scroll_offset(layout_width, &layout_lines, visible_height);
		}

		// Box-drawing characters
		let bx: &RoundedBoxSymbols = &self.symbols.box_round;
		let border_width = padding_x + 1;
		let top_left =
			(self.border_color)(&format!("{}{}", bx.top_left, bx.horizontal.repeat(padding_x)));
		let top_right =
			(self.border_color)(&format!("{}{}", bx.horizontal.repeat(padding_x), bx.top_right));
		let bottom_left = (self.border_color)(&format!(
			"{}{}{}",
			bx.bottom_left,
			bx.horizontal,
			make_padding(padding_x.saturating_sub(1))
		));
		let horizontal = (self.border_color)(bx.horizontal);

		// Layout text
		let layout_lines = layout_text(&self.state, layout_width);
		let visible_height = self.get_visible_content_height(layout_lines.len());
		let scroll = self
			.scroll_offset
			.min(layout_lines.len().saturating_sub(visible_height));
		let visible_layout_lines =
			&layout_lines[scroll..layout_lines.len().min(scroll + visible_height)];

		let mut result: Vec<String> = Vec::new();

		// Top border
		let top_fill_width = width.saturating_sub(border_width * 2);
		if let Some(ref top_border) = self.top_border_content {
			if top_border.width <= top_fill_width {
				let fill = top_fill_width - top_border.width;
				result.push(format!(
					"{top_left}{}{}{top_right}",
					top_border.content,
					(self.border_color)(&bx.horizontal.repeat(fill))
				));
			} else {
				let truncated = rho_text::truncate::truncate_to_width_str(
					&top_border.content,
					top_fill_width.saturating_sub(1),
					rho_text::truncate::EllipsisKind::None,
					false,
				)
				.unwrap_or_default();
				let trunc_width = rho_text::width::visible_width_str(&truncated);
				let fill = top_fill_width.saturating_sub(trunc_width);
				result.push(format!(
					"{top_left}{truncated}{}{top_right}",
					(self.border_color)(&bx.horizontal.repeat(fill))
				));
			}
		} else {
			result.push(format!("{top_left}{}{top_right}", horizontal.repeat(top_fill_width)));
		}

		// Render each layout line
		let emit_cursor_marker = self.focused && self.autocomplete_state.is_none();
		let line_content_width = content_area_width;
		let inline_hint = self.get_inline_hint();
		let default_hint_style: Box<dyn Fn(&str) -> String> =
			Box::new(|t: &str| format!("\x1b[2m{t}\x1b[0m"));
		let hint_style_fn: &dyn Fn(&str) -> String =
			self.hint_style.as_deref().unwrap_or(&*default_hint_style);

		for (idx, layout_line) in visible_layout_lines.iter().enumerate() {
			let mut display_text = layout_line.text.clone();
			let mut display_width = rho_text::width::visible_width_str(&display_text);
			let mut cursor_in_padding = false;

			let has_cursor = layout_line.has_cursor && layout_line.cursor_pos.is_some();
			let cursor_pos = layout_line.cursor_pos.unwrap_or(0);
			let marker = if emit_cursor_marker {
				CURSOR_MARKER
			} else {
				""
			};

			if has_cursor && self.use_terminal_cursor {
				if !marker.is_empty() {
					let before = &display_text[..cursor_pos.min(display_text.len())];
					let after = &display_text[cursor_pos.min(display_text.len())..];
					if after.is_empty() {
						if let Some(ref hint) = inline_hint {
							let avail = line_content_width.saturating_sub(display_width);
							let trunc_hint = rho_text::truncate::truncate_to_width_str(
								hint,
								avail,
								rho_text::truncate::EllipsisKind::None,
								false,
							)
							.unwrap_or_default();
							let styled_hint = hint_style_fn(&trunc_hint);
							display_text = format!("{before}{marker}{styled_hint}");
							display_width += rho_text::width::visible_width_str(hint).min(avail);
						} else {
							display_text = format!("{before}{marker}{after}");
						}
					} else {
						display_text = format!("{before}{marker}{after}");
					}
				}
			} else if has_cursor && !self.use_terminal_cursor {
				let before = display_text[..cursor_pos.min(display_text.len())].to_owned();
				let after = display_text[cursor_pos.min(display_text.len())..].to_owned();

				if !after.is_empty() {
					// Cursor on a character — replace with reverse video
					let first_grapheme = after.graphemes(true).next().unwrap_or("");
					let rest = &after[first_grapheme.len()..];
					let cursor = format!("\x1b[7m{first_grapheme}\x1b[0m");
					display_text = format!("{before}{marker}{cursor}{rest}");
				} else if let Some(ref cursor_override) = self.cursor_override {
					let override_width = self.cursor_override_width.unwrap_or(1);
					if let Some(ref hint) = inline_hint {
						let avail = line_content_width.saturating_sub(display_width + override_width);
						let trunc_hint = rho_text::truncate::truncate_to_width_str(
							hint,
							avail,
							rho_text::truncate::EllipsisKind::None,
							false,
						)
						.unwrap_or_default();
						let styled_hint = hint_style_fn(&trunc_hint);
						display_text = format!("{before}{marker}{cursor_override}{styled_hint}");
						display_width +=
							override_width + rho_text::width::visible_width_str(hint).min(avail);
					} else {
						display_text = format!("{before}{marker}{cursor_override}");
						display_width += override_width;
					}
				} else {
					// End-of-text cursor: blinking bar
					let cursor_char = self.symbols.input_cursor;
					let cursor = format!("\x1b[5m{cursor_char}\x1b[0m");
					let cursor_vis_w = rho_text::width::visible_width_str(cursor_char);
					if let Some(ref hint) = inline_hint {
						let avail = line_content_width.saturating_sub(display_width + cursor_vis_w);
						let trunc_hint = rho_text::truncate::truncate_to_width_str(
							hint,
							avail,
							rho_text::truncate::EllipsisKind::None,
							false,
						)
						.unwrap_or_default();
						let styled_hint = hint_style_fn(&trunc_hint);
						display_text = format!("{before}{marker}{cursor}{styled_hint}");
						display_width +=
							cursor_vis_w + rho_text::width::visible_width_str(hint).min(avail);
					} else {
						display_text = format!("{before}{marker}{cursor}");
						display_width += cursor_vis_w;
					}
					if display_width > line_content_width && padding_x > 0 {
						cursor_in_padding = true;
					}
				}
			}

			let is_last = idx == visible_layout_lines.len() - 1;
			let line_pad = make_padding(line_content_width.saturating_sub(display_width));

			let right_pad_width = if cursor_in_padding {
				padding_x.saturating_sub(1)
			} else {
				padding_x
			};

			if is_last {
				let bottom_right_pad = if cursor_in_padding {
					padding_x.saturating_sub(2)
				} else {
					padding_x.saturating_sub(1)
				};
				let bottom_right = (self.border_color)(&format!(
					"{}{}{}",
					make_padding(bottom_right_pad),
					bx.horizontal,
					bx.bottom_right,
				));
				result.push(format!("{bottom_left}{display_text}{line_pad}{bottom_right}"));
			} else {
				let left_border =
					(self.border_color)(&format!("{}{}", bx.vertical, make_padding(padding_x)));
				let right_border =
					(self.border_color)(&format!("{}{}", make_padding(right_pad_width), bx.vertical));
				result.push(format!("{left_border}{display_text}{line_pad}{right_border}"));
			}
		}

		// Add autocomplete list if active
		if self.autocomplete_state.is_some()
			&& let Some(ref mut list) = self.autocomplete_list
		{
			result.extend(list.render(width as u16));
		}

		result
	}

	#[allow(clippy::too_many_lines, reason = "input dispatch handles 40+ keybindings")]
	fn handle_input(&mut self, data: &str) -> InputResult {
		let kb = get_editor_keybindings();
		let lw = self.last_layout_width;

		// Handle character jump mode
		if let Some(jump_dir) = self.jump_mode {
			if kb.matches(data.as_bytes(), EditorAction::JumpForward, false)
				|| kb.matches(data.as_bytes(), EditorAction::JumpBackward, false)
			{
				self.jump_mode = None;
				return InputResult::Consumed;
			}

			if data.as_bytes().first().is_some_and(|&b| b >= 32) {
				let forward = jump_dir == JumpMode::Forward;
				self.jump_mode = None;
				jump_to_char(&mut self.state, data, forward, &mut self.preferred_visual_col);
				return InputResult::Consumed;
			}

			// Control character — cancel and fall through
			self.jump_mode = None;
		}

		// Handle bracketed paste mode
		if data.contains("\x1b[200~") {
			self.is_in_paste = true;
			self.paste_buffer.clear();
			let cleaned = data.replace("\x1b[200~", "");
			if !cleaned.is_empty() {
				self.paste_buffer.push_str(&cleaned);
			}
		}

		if self.is_in_paste {
			if !data.contains("\x1b[200~") {
				self.paste_buffer.push_str(data);
			}

			if let Some(end_idx) = self.paste_buffer.find("\x1b[201~") {
				let paste_content = self.paste_buffer[..end_idx].to_owned();
				let remaining = self.paste_buffer[end_idx + 6..].to_owned();

				self.handle_paste(&paste_content);
				self.is_in_paste = false;
				self.paste_buffer.clear();

				if !remaining.is_empty() {
					self.handle_input(&remaining);
				}
				return InputResult::Consumed;
			}
			return InputResult::Consumed;
		}

		// Ctrl+C — let parent handle
		if crate::keys::match_key::matches_key(data.as_bytes(), "ctrl+c", false) {
			return InputResult::Ignored;
		}

		// Undo (Ctrl+-)
		if kb.matches(data.as_bytes(), EditorAction::Undo, false) {
			self.apply_undo();
			return InputResult::Consumed;
		}

		// Handle autocomplete keys
		if self.autocomplete_state.is_some() && self.autocomplete_list.is_some() {
			if crate::keys::match_key::matches_key(data.as_bytes(), "escape", false)
				|| crate::keys::match_key::matches_key(data.as_bytes(), "esc", false)
			{
				self.cancel_autocomplete(true);
				return InputResult::Consumed;
			}

			if kb.matches(data.as_bytes(), EditorAction::SelectUp, false)
				|| kb.matches(data.as_bytes(), EditorAction::SelectDown, false)
			{
				if let Some(ref mut list) = self.autocomplete_list {
					list.handle_input(data);
				}
				if let Some(ref mut cb) = self.on_autocomplete_update {
					cb();
				}
				return InputResult::Consumed;
			}

			// Tab → apply completion
			if kb.matches(data.as_bytes(), EditorAction::Tab, false) {
				self.apply_autocomplete_selection();
				return InputResult::Consumed;
			}

			// Enter on slash command → apply autocomplete + submit immediately
			if (crate::keys::match_key::matches_key(data.as_bytes(), "enter", false)
				|| crate::keys::match_key::matches_key(data.as_bytes(), "return", false)
				|| data == "\n")
				&& self.autocomplete_prefix.starts_with('/')
			{
				// Check for stale autocomplete
				let current_before = self.state.text_before_cursor().to_owned();
				if current_before == self.autocomplete_prefix {
					self.apply_autocomplete_selection();
				} else {
					self.cancel_autocomplete(false);
				}
				if !self.disable_submit && let Some(text) = self.submit_value() {
					return InputResult::Submit(text);
				}
				return InputResult::Consumed;
			}
			// Enter on file path → apply and return
			else if crate::keys::match_key::matches_key(data.as_bytes(), "enter", false)
				|| crate::keys::match_key::matches_key(data.as_bytes(), "return", false)
				|| data == "\n"
			{
				self.apply_autocomplete_selection();
				return InputResult::Consumed;
			}
			// Other keys fall through to normal handling
		}

		// Tab (not in autocomplete)
		if kb.matches(data.as_bytes(), EditorAction::Tab, false) && self.autocomplete_state.is_none()
		{
			self.handle_tab_completion();
			return InputResult::Consumed;
		}

		// Emacs editing keys
		if kb.matches(data.as_bytes(), EditorAction::DeleteToLineEnd, false) {
			self.history.reset();
			self.record_undo_state();
			let deleted = editing::delete_to_end_of_line(&mut self.state);
			self.record_kill(&deleted, false);
			self.notify_change();
		} else if kb.matches(data.as_bytes(), EditorAction::DeleteToLineStart, false) {
			self.history.reset();
			self.record_undo_state();
			let deleted = editing::delete_to_start_of_line(&mut self.state);
			self.record_kill(&deleted, true);
			self.notify_change();
		} else if kb.matches(data.as_bytes(), EditorAction::DeleteWordBackward, false) {
			self.history.reset();
			self.record_undo_state();
			let deleted = editing::delete_word_backwards(&mut self.state);
			self.record_kill(&deleted, true);
			self.notify_change();
		} else if crate::keys::match_key::matches_key(data.as_bytes(), "alt+d", false)
			|| crate::keys::match_key::matches_key(data.as_bytes(), "alt+delete", false)
		{
			self.history.reset();
			self.record_undo_state();
			let deleted = editing::delete_word_forwards(&mut self.state);
			self.record_kill(&deleted, false);
			self.notify_change();
		} else if kb.matches(data.as_bytes(), EditorAction::Yank, false) {
			self.yank_from_kill_ring();
		} else if kb.matches(data.as_bytes(), EditorAction::YankPop, false) {
			self.yank_pop();
		} else if kb.matches(data.as_bytes(), EditorAction::CursorLineStart, false) {
			self.reset_kill_sequence();
			move_to_line_start(&mut self.state, &mut self.preferred_visual_col);
		} else if kb.matches(data.as_bytes(), EditorAction::CursorLineEnd, false) {
			self.reset_kill_sequence();
			move_to_line_end(&mut self.state, &mut self.preferred_visual_col);
		}
		// Alt+Enter
		else if crate::keys::match_key::matches_key(data.as_bytes(), "alt+enter", false) {
			if let Some(ref mut cb) = self.on_alt_enter {
				cb(&self.state.text());
			} else {
				self.history.reset();
				self.reset_kill_sequence();
				self.record_undo_state();
				editing::add_new_line(&mut self.state);
				set_cursor_col(&mut self.state, 0, &mut self.preferred_visual_col);
				self.notify_change();
			}
		}
		// New line (Shift+Enter, Ctrl+Enter, etc.)
		else if Self::is_new_line_key(data) {
			self.history.reset();
			self.reset_kill_sequence();
			self.record_undo_state();
			editing::add_new_line(&mut self.state);
			set_cursor_col(&mut self.state, 0, &mut self.preferred_visual_col);
			self.notify_change();
		}
		// Plain Enter — submit
		else if crate::keys::match_key::matches_key(data.as_bytes(), "enter", false)
			|| crate::keys::match_key::matches_key(data.as_bytes(), "return", false)
			|| data == "\n"
		{
			if !self.disable_submit && let Some(text) = self.submit_value() {
				return InputResult::Submit(text);
			}
		}
		// Backspace
		else if kb.matches(data.as_bytes(), EditorAction::DeleteCharBackward, false)
			|| crate::keys::match_key::matches_key(data.as_bytes(), "shift+backspace", false)
		{
			self.history.reset();
			self.reset_kill_sequence();
			self.record_undo_state();
			if editing::handle_backspace(&mut self.state) {
				// set_cursor_col clears preferred_visual_col
				self.preferred_visual_col = None;
				self.notify_change();
				if self.autocomplete_state.is_some() {
					self.update_autocomplete();
				} else {
					self.try_retrigger_autocomplete();
				}
			}
		}
		// Forward delete
		else if kb.matches(data.as_bytes(), EditorAction::DeleteCharForward, false)
			|| crate::keys::match_key::matches_key(data.as_bytes(), "shift+delete", false)
		{
			self.history.reset();
			self.reset_kill_sequence();
			self.record_undo_state();
			if editing::handle_forward_delete(&mut self.state) {
				self.notify_change();
				if self.autocomplete_state.is_some() {
					self.update_autocomplete();
				} else {
					self.try_retrigger_autocomplete();
				}
			}
		}
		// Word navigation
		else if kb.matches(data.as_bytes(), EditorAction::CursorWordLeft, false) {
			self.reset_kill_sequence();
			move_word_backwards(&mut self.state, &mut self.preferred_visual_col);
		} else if kb.matches(data.as_bytes(), EditorAction::CursorWordRight, false) {
			self.reset_kill_sequence();
			move_word_forwards(&mut self.state, &mut self.preferred_visual_col);
		}
		// Arrow Up
		else if kb.matches(data.as_bytes(), EditorAction::CursorUp, false) {
			if self.state.is_empty()
				|| (self.history.is_browsing() && is_on_first_visual_line(&self.state, lw))
			{
				self.navigate_history(-1);
			} else if is_on_first_visual_line(&self.state, lw) {
				move_to_line_start(&mut self.state, &mut self.preferred_visual_col);
			} else {
				move_cursor(&mut self.state, -1, 0, lw, &mut self.preferred_visual_col);
			}
		}
		// Arrow Down
		else if kb.matches(data.as_bytes(), EditorAction::CursorDown, false) {
			if self.history.is_browsing() && is_on_last_visual_line(&self.state, lw) {
				self.navigate_history(1);
			} else if is_on_last_visual_line(&self.state, lw) {
				move_to_line_end(&mut self.state, &mut self.preferred_visual_col);
			} else {
				move_cursor(&mut self.state, 1, 0, lw, &mut self.preferred_visual_col);
			}
		}
		// Arrow Right
		else if kb.matches(data.as_bytes(), EditorAction::CursorRight, false) {
			move_cursor(&mut self.state, 0, 1, lw, &mut self.preferred_visual_col);
		}
		// Arrow Left
		else if kb.matches(data.as_bytes(), EditorAction::CursorLeft, false) {
			move_cursor(&mut self.state, 0, -1, lw, &mut self.preferred_visual_col);
		}
		// Shift+Space → insert space
		else if crate::keys::match_key::matches_key(data.as_bytes(), "shift+space", false) {
			self.insert_character_internal(" ");
		}
		// Jump mode triggers
		else if kb.matches(data.as_bytes(), EditorAction::JumpForward, false) {
			self.jump_mode = Some(JumpMode::Forward);
		} else if kb.matches(data.as_bytes(), EditorAction::JumpBackward, false) {
			self.jump_mode = Some(JumpMode::Backward);
		}
		// Kitty CSI-u printable characters
		else if let Some(kitty_char) = Self::decode_kitty_printable(data) {
			self.insert_text(&kitty_char);
		}
		// Regular printable characters
		else if data.as_bytes().first().is_some_and(|&b| b >= 32) {
			self.insert_character_internal(data);
		} else {
			return InputResult::Ignored;
		}

		// Update scroll offset after any input
		let layout_lines = layout_text(&self.state, lw);
		let visible_height = self.get_visible_content_height(layout_lines.len());
		self.update_scroll_offset(lw, &layout_lines, visible_height);
		// Update last_layout_width
		self.last_layout_width = lw;

		InputResult::Consumed
	}

	fn invalidate(&mut self) {
		// No cached state to invalidate
	}
}

impl Focusable for Editor {
	fn set_focused(&mut self, focused: bool) {
		self.focused = focused;
	}

	fn is_focused(&self) -> bool {
		self.focused
	}
}

impl Editor {
	fn apply_autocomplete_selection(&mut self) {
		let Some(ref provider) = self.autocomplete_provider else {
			return;
		};

		let selected = self
			.autocomplete_list
			.as_ref()
			.and_then(|list| list.selected_item())
			.cloned();

		if let Some(selected) = selected {
			let prefix = self.autocomplete_prefix.clone();
			let result = provider.apply_completion(
				&self.state.lines,
				self.state.cursor_line,
				self.state.cursor_col,
				&selected,
				&prefix,
			);

			self.state.lines = result.lines;
			self.state.cursor_line = result.cursor_line;
			set_cursor_col(&mut self.state, result.cursor_col, &mut self.preferred_visual_col);

			self.cancel_autocomplete(false);
			self.notify_change();
		}
	}

	fn is_new_line_key(data: &str) -> bool {
		// Shift+Enter
		if crate::keys::match_key::matches_key(data.as_bytes(), "shift+enter", false) {
			return true;
		}
		// Ctrl+Enter (Kitty)
		if data == "\x1b[13;5u" || data == "\x1b[27;5;13~" {
			return true;
		}
		// Option+Enter (legacy)
		if data == "\x1b\r" {
			return true;
		}
		// Shift+Enter (legacy)
		if data == "\x1b[13;2~" {
			return true;
		}
		// Newline with modifiers
		if data.len() > 1 && data.as_bytes()[0] == 10 {
			return true;
		}
		false
	}

}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::symbols;

	fn test_symbols() -> SymbolTheme {
		symbols::SymbolTheme {
			cursor:         ">",
			input_cursor:   "|",
			box_round:      symbols::RoundedBoxSymbols {
				top_left:     "╭",
				top_right:    "╮",
				bottom_left:  "╰",
				bottom_right: "╯",
				horizontal:   "─",
				vertical:     "│",
			},
			box_sharp:      symbols::BoxSymbols {
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
			table:          symbols::BoxSymbols {
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
			tree:           symbols::TreeSymbols {
				branch:   "├─",
				last:     "╰─",
				vertical: "│",
			},
			quote_border:   "│",
			hr_char:        "─",
			spinner_frames: &["⠋"],
		}
	}

	fn plain_select_list_theme() -> SelectListTheme {
		SelectListTheme {
			selected_prefix: Box::new(|s| s.to_owned()),
			selected_text:   Box::new(|s| s.to_owned()),
			description:     Box::new(|s| s.to_owned()),
			scroll_info:     Box::new(|s| s.to_owned()),
			no_match:        Box::new(|s| s.to_owned()),
			symbols:         test_symbols(),
		}
	}

	fn make_editor() -> Editor {
		Editor::new(
			Box::new(|s| s.to_owned()),
			Box::new(plain_select_list_theme),
			test_symbols(),
			Some(2),
			None,
		)
	}

	// ── History navigation tests ────────────────────────────────────

	#[test]
	fn test_history_empty() {
		let mut editor = make_editor();
		editor.handle_input("\x1b[A"); // Up
		assert_eq!(editor.get_text(), "");
	}

	#[test]
	fn test_history_navigate_up() {
		let mut editor = make_editor();
		editor.add_to_history("first prompt");
		editor.add_to_history("second prompt");

		editor.handle_input("\x1b[A"); // Up
		assert_eq!(editor.get_text(), "second prompt");
	}

	#[test]
	fn test_history_cycle() {
		let mut editor = make_editor();
		editor.add_to_history("first");
		editor.add_to_history("second");
		editor.add_to_history("third");

		editor.handle_input("\x1b[A"); // third
		assert_eq!(editor.get_text(), "third");
		editor.handle_input("\x1b[A"); // second
		assert_eq!(editor.get_text(), "second");
		editor.handle_input("\x1b[A"); // first
		assert_eq!(editor.get_text(), "first");
		editor.handle_input("\x1b[A"); // stays at first
		assert_eq!(editor.get_text(), "first");
	}

	#[test]
	fn test_history_down_returns_empty() {
		let mut editor = make_editor();
		editor.add_to_history("prompt");
		editor.handle_input("\x1b[A"); // Up
		assert_eq!(editor.get_text(), "prompt");
		editor.handle_input("\x1b[B"); // Down
		assert_eq!(editor.get_text(), "");
	}

	#[test]
	fn test_history_exit_on_typing() {
		let mut editor = make_editor();
		editor.add_to_history("old prompt");
		editor.handle_input("\x1b[A"); // Up
		editor.handle_input("x");
		assert_eq!(editor.get_text(), "old promptx");
	}

	#[test]
	fn test_history_exit_on_set_text() {
		let mut editor = make_editor();
		editor.add_to_history("first");
		editor.add_to_history("second");
		editor.handle_input("\x1b[A"); // Up - shows "second"
		editor.set_text("");
		editor.handle_input("\x1b[A");
		assert_eq!(editor.get_text(), "second");
	}

	#[test]
	fn test_history_no_empty() {
		let mut editor = make_editor();
		editor.add_to_history("");
		editor.add_to_history("   ");
		editor.add_to_history("valid");
		editor.handle_input("\x1b[A");
		assert_eq!(editor.get_text(), "valid");
		editor.handle_input("\x1b[A");
		assert_eq!(editor.get_text(), "valid"); // no more entries
	}

	#[test]
	fn test_history_no_consecutive_dupes() {
		let mut editor = make_editor();
		editor.add_to_history("same");
		editor.add_to_history("same");
		editor.add_to_history("same");
		editor.handle_input("\x1b[A"); // same
		assert_eq!(editor.get_text(), "same");
		editor.handle_input("\x1b[A"); // stays
		assert_eq!(editor.get_text(), "same");
	}

	#[test]
	fn test_history_non_consecutive_dupes_allowed() {
		let mut editor = make_editor();
		editor.add_to_history("first");
		editor.add_to_history("second");
		editor.add_to_history("first");
		editor.handle_input("\x1b[A"); // first
		assert_eq!(editor.get_text(), "first");
		editor.handle_input("\x1b[A"); // second
		assert_eq!(editor.get_text(), "second");
		editor.handle_input("\x1b[A"); // first (older)
		assert_eq!(editor.get_text(), "first");
	}

	// ── State accessor tests ────────────────────────────────────────

	#[test]
	fn test_cursor_position() {
		let mut editor = make_editor();
		assert_eq!(editor.get_cursor(), (0, 0));
		editor.handle_input("a");
		editor.handle_input("b");
		editor.handle_input("c");
		assert_eq!(editor.get_cursor(), (0, 3));
		editor.handle_input("\x1b[D"); // Left
		assert_eq!(editor.get_cursor(), (0, 2));
	}

	#[test]
	fn test_get_lines_copy() {
		let mut editor = make_editor();
		editor.set_text("a\nb");
		let lines = editor.get_lines();
		assert_eq!(lines, vec!["a", "b"]);
	}

	// ── Unicode editing tests ───────────────────────────────────────

	#[test]
	fn test_insert_mixed_unicode() {
		let mut editor = make_editor();
		for ch in ["H", "e", "l", "l", "o", " ", "ä", "ö", "ü", " ", "😀"] {
			editor.handle_input(ch);
		}
		assert_eq!(editor.get_text(), "Hello äöü 😀");
	}

	#[test]
	fn test_backspace_umlaut() {
		let mut editor = make_editor();
		editor.handle_input("ä");
		editor.handle_input("ö");
		editor.handle_input("ü");
		editor.handle_input("\x7f"); // Backspace
		assert_eq!(editor.get_text(), "äö");
	}

	#[test]
	fn test_backspace_emoji() {
		let mut editor = make_editor();
		editor.handle_input("😀");
		editor.handle_input("👍");
		editor.handle_input("\x7f"); // Backspace
		assert_eq!(editor.get_text(), "😀");
	}

	#[test]
	fn test_cursor_move_over_umlaut() {
		let mut editor = make_editor();
		editor.handle_input("ä");
		editor.handle_input("ö");
		editor.handle_input("ü");
		editor.handle_input("\x1b[D"); // Left
		editor.handle_input("\x1b[D"); // Left
		editor.handle_input("x");
		assert_eq!(editor.get_text(), "äxöü");
	}

	#[test]
	fn test_cursor_move_over_emoji() {
		let mut editor = make_editor();
		editor.handle_input("😀");
		editor.handle_input("👍");
		editor.handle_input("🎉");
		editor.handle_input("\x1b[D"); // Left over 🎉
		editor.handle_input("\x1b[D"); // Left over 👍
		editor.handle_input("x");
		assert_eq!(editor.get_text(), "😀x👍🎉");
	}

	#[test]
	fn test_unicode_across_lines() {
		let mut editor = make_editor();
		editor.handle_input("ä");
		editor.handle_input("ö");
		editor.handle_input("ü");
		// Shift+Enter for new line
		editor.set_text("äöü\nÄÖÜ");
		assert_eq!(editor.get_text(), "äöü\nÄÖÜ");
	}

	#[test]
	fn test_set_text_unicode() {
		let mut editor = make_editor();
		editor.set_text("Hällö Wörld! 😀 äöüÄÖÜß");
		assert_eq!(editor.get_text(), "Hällö Wörld! 😀 äöüÄÖÜß");
	}

	#[test]
	fn test_ctrl_a_insert_at_start() {
		let mut editor = make_editor();
		editor.handle_input("a");
		editor.handle_input("b");
		editor.handle_input("\x01"); // Ctrl+A
		editor.handle_input("x");
		assert_eq!(editor.get_text(), "xab");
	}

	#[test]
	fn test_delete_word_backwards_basic() {
		let mut editor = make_editor();
		editor.set_text("foo bar baz");
		editor.handle_input("\x17"); // Ctrl+W
		assert_eq!(editor.get_text(), "foo bar ");
	}

	#[test]
	fn test_delete_word_backwards_trailing_space() {
		let mut editor = make_editor();
		editor.set_text("foo bar   ");
		editor.handle_input("\x17"); // Ctrl+W
		assert_eq!(editor.get_text(), "foo ");
	}

	#[test]
	fn test_delete_word_backwards_punctuation() {
		let mut editor = make_editor();
		editor.set_text("foo bar...");
		editor.handle_input("\x17"); // Ctrl+W
		assert_eq!(editor.get_text(), "foo bar");
	}

	// ── Sticky column tests ─────────────────────────────────────────

	#[test]
	fn test_sticky_column_up_through_short_line() {
		let mut editor = make_editor();
		editor.set_text("2222222222x222\n\n1111111111_111111111111");

		// Position at line 2, col 10
		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..10 {
			editor.handle_input("\x1b[C"); // Right
		}
		assert_eq!(editor.get_cursor(), (2, 10));

		editor.handle_input("\x1b[A"); // Up to empty line
		assert_eq!(editor.get_cursor(), (1, 0));

		editor.handle_input("\x1b[A"); // Up to line 0
		assert_eq!(editor.get_cursor(), (0, 10));
	}

	#[test]
	fn test_sticky_column_down_through_short_line() {
		let mut editor = make_editor();
		editor.set_text("1111111111_111\n\n2222222222x222222222222");

		// Go to line 0
		editor.handle_input("\x1b[A"); // Up to line 1
		editor.handle_input("\x1b[A"); // Up to line 0
		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..10 {
			editor.handle_input("\x1b[C"); // Right
		}
		assert_eq!(editor.get_cursor(), (0, 10));

		editor.handle_input("\x1b[B"); // Down to empty line
		assert_eq!(editor.get_cursor(), (1, 0));

		editor.handle_input("\x1b[B"); // Down to line 2
		assert_eq!(editor.get_cursor(), (2, 10));
	}

	#[test]
	fn test_sticky_column_reset_on_left() {
		let mut editor = make_editor();
		editor.set_text("1234567890\n\n1234567890");

		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..5 {
			editor.handle_input("\x1b[C");
		}
		assert_eq!(editor.get_cursor(), (2, 5));

		editor.handle_input("\x1b[A"); // Up
		editor.handle_input("\x1b[A"); // Up - line 0, col 5
		assert_eq!(editor.get_cursor(), (0, 5));

		editor.handle_input("\x1b[D"); // Left
		assert_eq!(editor.get_cursor(), (0, 4));

		editor.handle_input("\x1b[B"); // Down
		editor.handle_input("\x1b[B"); // Down - line 2, col 4
		assert_eq!(editor.get_cursor(), (2, 4));
	}

	#[test]
	fn test_sticky_column_reset_on_typing() {
		let mut editor = make_editor();
		editor.set_text("1234567890\n\n1234567890");

		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..8 {
			editor.handle_input("\x1b[C");
		}

		editor.handle_input("\x1b[A"); // Up
		editor.handle_input("\x1b[A"); // Up - line 0, col 8
		assert_eq!(editor.get_cursor(), (0, 8));

		editor.handle_input("X");
		assert_eq!(editor.get_cursor(), (0, 9));

		editor.handle_input("\x1b[B"); // Down
		editor.handle_input("\x1b[B"); // Down - line 2, col 9
		assert_eq!(editor.get_cursor(), (2, 9));
	}

	#[test]
	fn test_sticky_column_reset_on_backspace() {
		let mut editor = make_editor();
		editor.set_text("1234567890\n\n1234567890");

		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..8 {
			editor.handle_input("\x1b[C");
		}

		editor.handle_input("\x1b[A"); // Up
		editor.handle_input("\x1b[A"); // Up - line 0, col 8
		assert_eq!(editor.get_cursor(), (0, 8));

		editor.handle_input("\x7f"); // Backspace
		assert_eq!(editor.get_cursor(), (0, 7));

		editor.handle_input("\x1b[B"); // Down
		editor.handle_input("\x1b[B"); // Down - line 2, col 7
		assert_eq!(editor.get_cursor(), (2, 7));
	}

	#[test]
	fn test_sticky_column_reset_on_ctrl_a() {
		let mut editor = make_editor();
		editor.set_text("1234567890\n\n1234567890");

		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..8 {
			editor.handle_input("\x1b[C");
		}
		editor.handle_input("\x1b[A"); // Up - establishes sticky
		editor.handle_input("\x01"); // Ctrl+A - resets
		assert_eq!(editor.get_cursor(), (1, 0));

		editor.handle_input("\x1b[A"); // Up
		assert_eq!(editor.get_cursor(), (0, 0));
	}

	#[test]
	fn test_sticky_column_reset_on_ctrl_e() {
		let mut editor = make_editor();
		editor.set_text("12345\n\n1234567890");

		editor.handle_input("\x01"); // Ctrl+A
		for _ in 0..3 {
			editor.handle_input("\x1b[C");
		}
		editor.handle_input("\x1b[A"); // Up
		editor.handle_input("\x1b[A"); // Up - line 0, col 3
		assert_eq!(editor.get_cursor(), (0, 3));

		editor.handle_input("\x05"); // Ctrl+E
		assert_eq!(editor.get_cursor(), (0, 5));

		editor.handle_input("\x1b[B"); // Down
		editor.handle_input("\x1b[B"); // Down - line 2, col 5
		assert_eq!(editor.get_cursor(), (2, 5));
	}

	#[test]
	fn test_history_limit_100() {
		let mut editor = make_editor();
		for i in 0..105 {
			editor.add_to_history(&format!("prompt {i}"));
		}
		// Navigate to oldest
		for _ in 0..100 {
			editor.handle_input("\x1b[A");
		}
		assert_eq!(editor.get_text(), "prompt 5");
		editor.handle_input("\x1b[A"); // no further
		assert_eq!(editor.get_text(), "prompt 5");
	}

	// ── Render tests ────────────────────────────────────────────────

	#[test]
	fn test_render_basic() {
		let mut editor = make_editor();
		editor.set_focused(true);
		let lines = editor.render(40);
		// Should have at least top border + one content line
		assert!(lines.len() >= 2);
		// All lines should fit width
		for line in &lines {
			assert!(rho_text::width::visible_width_str(line) <= 40, "line exceeds width: {line}");
		}
	}

	#[test]
	fn test_render_with_text() {
		let mut editor = make_editor();
		editor.set_text("Hello");
		let lines = editor.render(40);
		let joined = lines.join("\n");
		assert!(joined.contains("Hello"));
	}

	#[test]
	fn test_word_nav_ctrl_left_right() {
		let mut editor = make_editor();
		editor.set_text("foo bar... baz");

		editor.handle_input("\x1b[1;5D"); // Ctrl+Left
		assert_eq!(editor.get_cursor(), (0, 11));

		editor.handle_input("\x1b[1;5D"); // Ctrl+Left
		assert_eq!(editor.get_cursor(), (0, 7));

		editor.handle_input("\x1b[1;5D"); // Ctrl+Left
		assert_eq!(editor.get_cursor(), (0, 4));

		editor.handle_input("\x1b[1;5C"); // Ctrl+Right
		assert_eq!(editor.get_cursor(), (0, 7));

		editor.handle_input("\x1b[1;5C"); // Ctrl+Right
		assert_eq!(editor.get_cursor(), (0, 10));

		editor.handle_input("\x1b[1;5C"); // Ctrl+Right
		assert_eq!(editor.get_cursor(), (0, 14));
	}

	#[test]
	fn test_cursor_uses_content_not_history() {
		let mut editor = make_editor();
		editor.add_to_history("history item");
		editor.set_text("line1\nline2");

		editor.handle_input("\x1b[A"); // Up - cursor movement
		editor.handle_input("X");
		assert_eq!(editor.get_text(), "line1X\nline2");
	}
}
