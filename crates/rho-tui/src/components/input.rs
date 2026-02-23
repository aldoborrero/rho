//! Input component — single-line text input with horizontal scrolling.
//!
//! Features: grapheme-aware cursor movement, Emacs-style kill ring,
//! word boundary navigation, undo, and bracketed paste.

use unicode_segmentation::UnicodeSegmentation;

use super::text::make_padding;
use crate::{
	component::{CURSOR_MARKER, Component, Focusable, InputResult},
	keybindings::{EditorAction, get_editor_keybindings},
	kill_ring::KillRing,
};

/// Callback type for submit events.
pub type OnSubmit = Box<dyn FnMut(&str)>;

/// Tracked action for kill-ring accumulation and undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastAction {
	Kill,
	Yank,
	TypeWord,
}

/// Snapshot for undo.
#[derive(Debug, Clone)]
struct InputState {
	value:  String,
	cursor: usize,
}

/// Single-line text input with horizontal scrolling.
pub struct Input {
	value:   String,
	/// Cursor position as byte offset into value.
	cursor:  usize,
	focused: bool,

	// Callbacks
	pub on_submit: Option<OnSubmit>,
	pub on_escape: Option<Box<dyn FnMut()>>,

	// Bracketed paste
	paste_buffer: String,
	is_in_paste:  bool,

	// Kill ring
	kill_ring:   KillRing,
	last_action: Option<LastAction>,

	// Undo
	undo_stack: Vec<InputState>,
}

impl Input {
	pub fn new() -> Self {
		Self {
			value:        String::new(),
			cursor:       0,
			focused:      false,
			on_submit:    None,
			on_escape:    None,
			paste_buffer: String::new(),
			is_in_paste:  false,
			kill_ring:    KillRing::new(),
			last_action:  None,
			undo_stack:   Vec::new(),
		}
	}

	pub fn value(&self) -> &str {
		&self.value
	}

	pub fn set_value(&mut self, value: &str) {
		value.clone_into(&mut self.value);
		self.cursor = self.cursor.min(self.value.len());
	}

	// ── Character classification ─────────────────────────────────────

	fn is_whitespace_char(grapheme: &str) -> bool {
		grapheme
			.chars()
			.next()
			.is_some_and(|c| matches!(c, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' '))
	}

	fn is_punctuation_char(grapheme: &str) -> bool {
		grapheme.chars().next().is_some_and(|c| {
			matches!(
				c,
				'(' | ')'
					| '{' | '}'
					| '[' | ']'
					| '<' | '>'
					| '.' | ','
					| ';' | ':'
					| '\'' | '"'
					| '!' | '?'
					| '+' | '-'
					| '=' | '*'
					| '/' | '\\'
					| '|' | '&'
					| '%' | '^'
					| '$' | '#'
					| '@' | '~'
					| '`'
			)
		})
	}

	fn has_control_chars(data: &str) -> bool {
		data.chars().any(|c| {
			let code = c as u32;
			code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
		})
	}

	// ── Text editing primitives ──────────────────────────────────────

	fn insert_text(&mut self, text: &str) {
		let is_word_chunk = text.graphemes(true).all(|g| !Self::is_whitespace_char(g));
		// Undo coalescing: consecutive word typing coalesces into one undo unit.
		if !is_word_chunk || self.last_action != Some(LastAction::TypeWord) {
			self.push_undo();
		}
		self.last_action = Some(LastAction::TypeWord);

		self.value.insert_str(self.cursor, text);
		self.cursor += text.len();
	}

	fn handle_backspace(&mut self) {
		self.last_action = None;
		if self.cursor == 0 {
			return;
		}

		self.push_undo();

		let before_cursor = &self.value[..self.cursor];
		let graphemes: Vec<&str> = before_cursor.graphemes(true).collect();
		let last_grapheme = graphemes.last().copied().unwrap_or("");
		let grapheme_len = last_grapheme.len().max(1);

		let new_cursor = self.cursor - grapheme_len;
		self.value = format!("{}{}", &self.value[..new_cursor], &self.value[self.cursor..]);
		self.cursor = new_cursor;
	}

	fn handle_forward_delete(&mut self) {
		self.last_action = None;
		if self.cursor >= self.value.len() {
			return;
		}

		self.push_undo();

		let after_cursor = &self.value[self.cursor..];
		let first_grapheme = after_cursor.graphemes(true).next().unwrap_or("");
		let grapheme_len = first_grapheme.len().max(1);

		self.value = format!(
			"{}{}",
			&self.value[..self.cursor],
			&self.value[(self.cursor + grapheme_len).min(self.value.len())..]
		);
	}

	fn delete_to_line_start(&mut self) {
		if self.cursor == 0 {
			return;
		}

		self.push_undo();
		let deleted_text = self.value[..self.cursor].to_owned();
		self
			.kill_ring
			.push(&deleted_text, true, self.last_action == Some(LastAction::Kill));
		self.last_action = Some(LastAction::Kill);

		self.value = self.value[self.cursor..].to_owned();
		self.cursor = 0;
	}

	fn delete_to_line_end(&mut self) {
		if self.cursor >= self.value.len() {
			return;
		}

		self.push_undo();
		let deleted_text = self.value[self.cursor..].to_owned();
		self
			.kill_ring
			.push(&deleted_text, false, self.last_action == Some(LastAction::Kill));
		self.last_action = Some(LastAction::Kill);

		self.value = self.value[..self.cursor].to_owned();
	}

	fn delete_word_backwards(&mut self) {
		if self.cursor == 0 {
			return;
		}

		let was_kill = self.last_action == Some(LastAction::Kill);
		self.push_undo();

		let old_cursor = self.cursor;
		self.move_word_backwards();
		let delete_from = self.cursor;
		self.cursor = old_cursor;

		let deleted_text = self.value[delete_from..self.cursor].to_owned();
		self.kill_ring.push(&deleted_text, true, was_kill);
		self.last_action = Some(LastAction::Kill);

		self.value = format!("{}{}", &self.value[..delete_from], &self.value[self.cursor..]);
		self.cursor = delete_from;
	}

	fn delete_word_forward(&mut self) {
		if self.cursor >= self.value.len() {
			return;
		}

		let was_kill = self.last_action == Some(LastAction::Kill);
		self.push_undo();

		let old_cursor = self.cursor;
		self.move_word_forwards();
		let delete_to = self.cursor;
		self.cursor = old_cursor;

		let deleted_text = self.value[self.cursor..delete_to].to_owned();
		self.kill_ring.push(&deleted_text, false, was_kill);
		self.last_action = Some(LastAction::Kill);

		self.value = format!("{}{}", &self.value[..self.cursor], &self.value[delete_to..]);
	}

	fn yank(&mut self) {
		let text = match self.kill_ring.peek() {
			Some(t) => t.to_owned(),
			None => return,
		};

		self.push_undo();
		self.value.insert_str(self.cursor, &text);
		self.cursor += text.len();
		self.last_action = Some(LastAction::Yank);
	}

	fn yank_pop(&mut self) {
		if self.last_action != Some(LastAction::Yank) || self.kill_ring.len() <= 1 {
			return;
		}

		self.push_undo();

		let prev_text = self.kill_ring.peek().unwrap_or("").to_owned();
		let prev_len = prev_text.len();
		let remove_start = self.cursor.saturating_sub(prev_len);
		self.value = format!("{}{}", &self.value[..remove_start], &self.value[self.cursor..]);
		self.cursor = remove_start;

		self.kill_ring.rotate();
		let text = self.kill_ring.peek().unwrap_or("").to_owned();
		self.value.insert_str(self.cursor, &text);
		self.cursor += text.len();
		self.last_action = Some(LastAction::Yank);
	}

	// ── Undo ─────────────────────────────────────────────────────────

	fn push_undo(&mut self) {
		self
			.undo_stack
			.push(InputState { value: self.value.clone(), cursor: self.cursor });
	}

	fn undo(&mut self) {
		if let Some(snapshot) = self.undo_stack.pop() {
			self.value = snapshot.value;
			self.cursor = snapshot.cursor;
			self.last_action = None;
		}
	}

	// ── Cursor movement ──────────────────────────────────────────────

	fn move_cursor_left(&mut self) {
		self.last_action = None;
		if self.cursor == 0 {
			return;
		}

		let before_cursor = &self.value[..self.cursor];
		let last_grapheme = before_cursor.graphemes(true).next_back().unwrap_or("");
		self.cursor -= last_grapheme.len().max(1);
	}

	fn move_cursor_right(&mut self) {
		self.last_action = None;
		if self.cursor >= self.value.len() {
			return;
		}

		let after_cursor = &self.value[self.cursor..];
		let first_grapheme = after_cursor.graphemes(true).next().unwrap_or("");
		self.cursor += first_grapheme.len().max(1);
	}

	fn move_word_backwards(&mut self) {
		if self.cursor == 0 {
			return;
		}
		self.last_action = None;

		let before = &self.value[..self.cursor];
		let graphemes: Vec<&str> = before.graphemes(true).collect();
		let mut i = graphemes.len();

		// Skip trailing whitespace
		while i > 0 && Self::is_whitespace_char(graphemes[i - 1]) {
			self.cursor -= graphemes[i - 1].len();
			i -= 1;
		}

		if i > 0 {
			if Self::is_punctuation_char(graphemes[i - 1]) {
				// Skip punctuation run
				while i > 0 && Self::is_punctuation_char(graphemes[i - 1]) {
					self.cursor -= graphemes[i - 1].len();
					i -= 1;
				}
			} else {
				// Skip word run
				while i > 0
					&& !Self::is_whitespace_char(graphemes[i - 1])
					&& !Self::is_punctuation_char(graphemes[i - 1])
				{
					self.cursor -= graphemes[i - 1].len();
					i -= 1;
				}
			}
		}
	}

	fn move_word_forwards(&mut self) {
		if self.cursor >= self.value.len() {
			return;
		}
		self.last_action = None;

		let after = &self.value[self.cursor..];
		let mut graphemes = after.graphemes(true);

		// Skip leading whitespace
		while let Some(g) = graphemes.clone().next() {
			if !Self::is_whitespace_char(g) {
				break;
			}
			self.cursor += g.len();
			graphemes.next();
		}

		if let Some(first) = graphemes.clone().next() {
			if Self::is_punctuation_char(first) {
				// Skip punctuation run
				while let Some(g) = graphemes.clone().next() {
					if !Self::is_punctuation_char(g) {
						break;
					}
					self.cursor += g.len();
					graphemes.next();
				}
			} else {
				// Skip word run
				while let Some(g) = graphemes.clone().next() {
					if Self::is_whitespace_char(g) || Self::is_punctuation_char(g) {
						break;
					}
					self.cursor += g.len();
					graphemes.next();
				}
			}
		}
	}

	// ── Paste ────────────────────────────────────────────────────────

	fn handle_paste(&mut self, pasted_text: &str) {
		self.last_action = None;
		self.push_undo();

		// Clean pasted text — remove newlines
		let clean_text = pasted_text.replace("\r\n", "").replace(['\r', '\n'], "");

		self.value.insert_str(self.cursor, &clean_text);
		self.cursor += clean_text.len();
	}
}

impl Default for Input {
	fn default() -> Self {
		Self::new()
	}
}

impl Focusable for Input {
	fn set_focused(&mut self, focused: bool) {
		self.focused = focused;
	}

	fn is_focused(&self) -> bool {
		self.focused
	}
}

impl Component for Input {
	fn render(&mut self, width: u16) -> Vec<String> {
		let prompt = "> ";
		let w = width as usize;
		let available_width = w.saturating_sub(prompt.len());

		if available_width == 0 {
			return vec![prompt.to_owned()];
		}

		let visible_text;
		let cursor_display;

		let value_vis_width = rho_text::width::visible_width_str(&self.value);
		if value_vis_width < available_width {
			// Everything fits
			visible_text = self.value.clone();
			cursor_display = self.cursor;
		} else {
			// Need horizontal scrolling
			let scroll_width = if self.cursor == self.value.len() {
				available_width - 1
			} else {
				available_width
			};
			let half_width = scroll_width / 2;

			if self.cursor < half_width {
				// Cursor near start
				let end = find_grapheme_boundary(&self.value, scroll_width);
				visible_text = self.value[..end].to_owned();
				cursor_display = self.cursor;
			} else if self.cursor > self.value.len().saturating_sub(half_width) {
				// Cursor near end
				let start = find_grapheme_boundary_from_end(&self.value, scroll_width);
				visible_text = self.value[start..].to_owned();
				cursor_display = self.cursor - start;
			} else {
				// Cursor in middle
				let start = find_grapheme_boundary(&self.value, self.cursor - half_width);
				let end = find_grapheme_boundary(&self.value, start + scroll_width);
				visible_text = self.value[start..end.min(self.value.len())].to_owned();
				cursor_display = self.cursor - start;
			}
		}

		// Build line with cursor display
		let after_cursor_text = &visible_text[cursor_display.min(visible_text.len())..];
		let cursor_grapheme = after_cursor_text.graphemes(true).next().unwrap_or(" ");
		let at_cursor = if cursor_grapheme == " " && cursor_display >= visible_text.len() {
			" "
		} else {
			cursor_grapheme
		};

		let before_cursor = &visible_text[..cursor_display.min(visible_text.len())];
		let after_cursor =
			&visible_text[(cursor_display + at_cursor.len()).min(visible_text.len())..];

		// Hardware cursor marker for IME positioning
		let marker = if self.focused { CURSOR_MARKER } else { "" };

		// Inverse video cursor: ESC[7m = reverse, ESC[27m = normal
		let cursor_char = format!("\x1b[7m{at_cursor}\x1b[27m");
		let text_with_cursor = format!("{before_cursor}{marker}{cursor_char}{after_cursor}");

		// Calculate visual width and pad
		let visual_length = rho_text::width::visible_width_str(&text_with_cursor);
		let pad = make_padding(available_width.saturating_sub(visual_length));
		let line = format!("{prompt}{text_with_cursor}{pad}");

		vec![line]
	}

	fn handle_input(&mut self, data: &str) -> InputResult {
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

			if let Some(end_index) = self.paste_buffer.find("\x1b[201~") {
				let paste_content = self.paste_buffer[..end_index].to_owned();
				let remaining = self.paste_buffer[end_index + 6..].to_owned();

				self.handle_paste(&paste_content);
				self.is_in_paste = false;
				self.paste_buffer.clear();

				if !remaining.is_empty() {
					self.handle_input(&remaining);
				}
			}
			return InputResult::Consumed;
		}

		let bytes = data.as_bytes();
		let kb = get_editor_keybindings();

		// Escape/Cancel
		if kb.matches(bytes, EditorAction::SelectCancel, false) {
			if let Some(ref mut cb) = self.on_escape {
				cb();
			}
			return InputResult::Consumed;
		}

		// Undo
		if kb.matches(bytes, EditorAction::Undo, false) {
			self.undo();
			return InputResult::Consumed;
		}

		// Submit
		if kb.matches(bytes, EditorAction::Submit, false) || data == "\n" {
			if let Some(ref mut cb) = self.on_submit {
				cb(&self.value);
			}
			return InputResult::Consumed;
		}

		// Deletion
		if kb.matches(bytes, EditorAction::DeleteCharBackward, false) {
			self.handle_backspace();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::DeleteCharForward, false) {
			self.handle_forward_delete();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::DeleteWordBackward, false) {
			self.delete_word_backwards();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::DeleteWordForward, false) {
			self.delete_word_forward();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::DeleteToLineStart, false) {
			self.delete_to_line_start();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::DeleteToLineEnd, false) {
			self.delete_to_line_end();
			return InputResult::Consumed;
		}

		// Kill ring
		if kb.matches(bytes, EditorAction::Yank, false) {
			self.yank();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::YankPop, false) {
			self.yank_pop();
			return InputResult::Consumed;
		}

		// Cursor movement
		if kb.matches(bytes, EditorAction::CursorLeft, false) {
			self.move_cursor_left();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::CursorRight, false) {
			self.move_cursor_right();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::CursorLineStart, false) {
			self.last_action = None;
			self.cursor = 0;
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::CursorLineEnd, false) {
			self.last_action = None;
			self.cursor = self.value.len();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::CursorWordLeft, false) {
			self.move_word_backwards();
			return InputResult::Consumed;
		}
		if kb.matches(bytes, EditorAction::CursorWordRight, false) {
			self.move_word_forwards();
			return InputResult::Consumed;
		}

		// Regular character input — reject control characters
		if !Self::has_control_chars(data) {
			self.insert_text(data);
			return InputResult::Consumed;
		}

		InputResult::Ignored
	}
}

/// Find the byte position closest to `target_byte_offset` that falls on a
/// grapheme boundary. Clamps to string length.
fn find_grapheme_boundary(s: &str, target_byte_offset: usize) -> usize {
	if target_byte_offset >= s.len() {
		return s.len();
	}
	// Walk graphemes until we reach or pass the target
	let mut pos = 0;
	for g in s.graphemes(true) {
		if pos + g.len() > target_byte_offset {
			return pos;
		}
		pos += g.len();
	}
	pos
}

/// Find a byte position `scroll_width` bytes from the end, snapped to a
/// grapheme boundary.
fn find_grapheme_boundary_from_end(s: &str, scroll_width: usize) -> usize {
	if scroll_width >= s.len() {
		return 0;
	}
	let target = s.len() - scroll_width;
	find_grapheme_boundary(s, target)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_input_initial_state() {
		let input = Input::new();
		assert_eq!(input.value(), "");
		assert_eq!(input.cursor, 0);
	}

	#[test]
	fn test_input_set_value() {
		let mut input = Input::new();
		input.set_value("hello");
		assert_eq!(input.value(), "hello");
		// Cursor should be clamped
		input.cursor = 10;
		input.set_value("hi");
		assert_eq!(input.cursor, 2);
	}

	#[test]
	fn test_input_insert_text() {
		let mut input = Input::new();
		input.insert_text("hello");
		assert_eq!(input.value(), "hello");
		assert_eq!(input.cursor, 5);

		input.insert_text(" world");
		assert_eq!(input.value(), "hello world");
		assert_eq!(input.cursor, 11);
	}

	#[test]
	fn test_input_insert_at_cursor() {
		let mut input = Input::new();
		input.insert_text("helloworld");
		input.cursor = 5;
		input.insert_text(" ");
		assert_eq!(input.value(), "hello world");
		assert_eq!(input.cursor, 6);
	}

	#[test]
	fn test_input_backspace() {
		let mut input = Input::new();
		input.insert_text("hello");
		input.handle_backspace();
		assert_eq!(input.value(), "hell");
		assert_eq!(input.cursor, 4);
	}

	#[test]
	fn test_input_backspace_at_start() {
		let mut input = Input::new();
		input.insert_text("hello");
		input.cursor = 0;
		input.handle_backspace();
		assert_eq!(input.value(), "hello");
		assert_eq!(input.cursor, 0);
	}

	#[test]
	fn test_input_forward_delete() {
		let mut input = Input::new();
		input.insert_text("hello");
		input.cursor = 0;
		input.handle_forward_delete();
		assert_eq!(input.value(), "ello");
		assert_eq!(input.cursor, 0);
	}

	#[test]
	fn test_input_forward_delete_at_end() {
		let mut input = Input::new();
		input.insert_text("hello");
		input.handle_forward_delete();
		assert_eq!(input.value(), "hello");
	}

	#[test]
	fn test_input_cursor_movement() {
		let mut input = Input::new();
		input.insert_text("hello");
		assert_eq!(input.cursor, 5);

		input.move_cursor_left();
		assert_eq!(input.cursor, 4);

		input.move_cursor_right();
		assert_eq!(input.cursor, 5);

		// At end, right should not move
		input.move_cursor_right();
		assert_eq!(input.cursor, 5);

		// Go to start
		input.cursor = 0;
		input.move_cursor_left();
		assert_eq!(input.cursor, 0);
	}

	#[test]
	fn test_input_word_movement() {
		let mut input = Input::new();
		input.insert_text("hello world foo");
		input.cursor = input.value.len();

		// Move backward one word
		input.move_word_backwards();
		assert_eq!(input.cursor, 12); // before "foo"

		input.move_word_backwards();
		assert_eq!(input.cursor, 6); // before "world"

		input.move_word_backwards();
		assert_eq!(input.cursor, 0); // before "hello"

		// Move forward
		input.move_word_forwards();
		assert_eq!(input.cursor, 5); // after "hello"

		input.move_word_forwards();
		assert_eq!(input.cursor, 11); // after "world"
	}

	#[test]
	fn test_input_word_movement_with_punctuation() {
		let mut input = Input::new();
		input.insert_text("hello.world");
		input.cursor = input.value.len();

		// Move backward: should stop at punctuation boundary
		input.move_word_backwards();
		assert_eq!(input.cursor, 6); // after "."

		input.move_word_backwards();
		assert_eq!(input.cursor, 5); // at "."

		input.move_word_backwards();
		assert_eq!(input.cursor, 0); // before "hello"
	}

	#[test]
	fn test_input_delete_word_backwards() {
		let mut input = Input::new();
		input.insert_text("hello world");
		input.delete_word_backwards();
		assert_eq!(input.value(), "hello ");
	}

	#[test]
	fn test_input_delete_word_forward() {
		let mut input = Input::new();
		input.insert_text("hello world");
		input.cursor = 0;
		input.delete_word_forward();
		assert_eq!(input.value(), " world");
	}

	#[test]
	fn test_input_delete_to_line_start() {
		let mut input = Input::new();
		input.insert_text("hello world");
		input.cursor = 5;
		input.delete_to_line_start();
		assert_eq!(input.value(), " world");
		assert_eq!(input.cursor, 0);
	}

	#[test]
	fn test_input_delete_to_line_end() {
		let mut input = Input::new();
		input.insert_text("hello world");
		input.cursor = 5;
		input.delete_to_line_end();
		assert_eq!(input.value(), "hello");
		assert_eq!(input.cursor, 5);
	}

	#[test]
	fn test_input_undo() {
		let mut input = Input::new();
		input.insert_text("hello");
		input.push_undo();
		input.insert_text(" world");
		assert_eq!(input.value(), "hello world");

		input.undo();
		assert_eq!(input.value(), "hello");
	}

	#[test]
	fn test_input_yank() {
		let mut input = Input::new();
		input.insert_text("hello world");
		input.cursor = 5; // position cursor after "hello"
		input.delete_to_line_end(); // kills " world"
		assert_eq!(input.value(), "hello");

		// Yank it back
		input.yank();
		assert_eq!(input.value(), "hello world");
	}

	#[test]
	fn test_input_kill_accumulation() {
		let mut input = Input::new();
		input.insert_text("abc def");
		// Kill to end: kills " def" but cursor is at end so nothing
		input.cursor = 0;
		input.delete_to_line_end();
		assert_eq!(input.value(), "");

		// The kill ring should have "abc def"
		assert_eq!(input.kill_ring.peek(), Some("abc def"));
	}

	#[test]
	fn test_input_render_basic() {
		let mut input = Input::new();
		input.insert_text("hello");
		let lines = input.render(40);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].starts_with("> "));
		// Should contain "hello" somewhere
		assert!(lines[0].contains("hell"));
	}

	#[test]
	fn test_input_render_empty() {
		let mut input = Input::new();
		let lines = input.render(40);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].starts_with("> "));
	}

	#[test]
	fn test_input_paste() {
		let mut input = Input::new();
		input.handle_paste("hello\nworld");
		assert_eq!(input.value(), "helloworld");
	}

	#[test]
	fn test_input_paste_crlf() {
		let mut input = Input::new();
		input.handle_paste("hello\r\nworld\r\nfoo");
		assert_eq!(input.value(), "helloworldfoo");
	}

	#[test]
	fn test_has_control_chars() {
		assert!(Input::has_control_chars("\x1b"));
		assert!(Input::has_control_chars("\x01"));
		assert!(!Input::has_control_chars("hello"));
		assert!(!Input::has_control_chars("hello world 123"));
	}

	#[test]
	fn test_input_handle_submit() {
		let mut submitted = false;
		let submitted_ref = &mut submitted as *mut bool;
		let mut input = Input::new();
		input.on_submit = Some(Box::new(move |_| unsafe {
			*submitted_ref = true;
		}));
		input.handle_input("\r");
		assert!(submitted);
	}

	#[test]
	fn test_input_handle_escape() {
		let mut escaped = false;
		let escaped_ref = &mut escaped as *mut bool;
		let mut input = Input::new();
		input.on_escape = Some(Box::new(move || unsafe {
			*escaped_ref = true;
		}));
		input.handle_input("\x1b");
		assert!(escaped);
	}

	#[test]
	fn test_grapheme_boundary() {
		assert_eq!(find_grapheme_boundary("hello", 3), 3);
		assert_eq!(find_grapheme_boundary("hello", 10), 5);
		assert_eq!(find_grapheme_boundary("hello", 0), 0);
	}

	#[test]
	fn test_grapheme_boundary_from_end() {
		assert_eq!(find_grapheme_boundary_from_end("hello", 3), 2);
		assert_eq!(find_grapheme_boundary_from_end("hello", 10), 0);
	}
}
