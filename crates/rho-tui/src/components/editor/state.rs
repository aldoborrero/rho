//! Editor state — core data model for the multi-line editor.

/// Core editor state: lines of text and cursor position.
#[derive(Debug, Clone)]
pub struct EditorState {
	pub lines:       Vec<String>,
	pub cursor_line: usize,
	pub cursor_col:  usize,
}

impl Default for EditorState {
	fn default() -> Self {
		Self { lines: vec![String::new()], cursor_line: 0, cursor_col: 0 }
	}
}

impl EditorState {
	/// Get the full text by joining lines with newlines.
	pub fn text(&self) -> String {
		self.lines.join("\n")
	}

	/// Whether the editor is empty (single empty line).
	pub fn is_empty(&self) -> bool {
		self.lines.len() == 1 && self.lines[0].is_empty()
	}

	/// Get the current line text.
	pub fn current_line(&self) -> &str {
		self.lines.get(self.cursor_line).map_or("", String::as_str)
	}

	/// Get the text before the cursor on the current line.
	pub fn text_before_cursor(&self) -> &str {
		let line = self.current_line();
		&line[..self.cursor_col.min(line.len())]
	}

	/// Get the text after the cursor on the current line.
	pub fn text_after_cursor(&self) -> &str {
		let line = self.current_line();
		&line[self.cursor_col.min(line.len())..]
	}
}

/// A visual line entry mapping visual rows back to logical lines.
#[derive(Debug, Clone)]
pub struct VisualLine {
	/// Index into `EditorState.lines`.
	pub logical_line: usize,
	/// Starting byte offset in the logical line.
	pub start_col:    usize,
	/// Byte length of this visual line segment.
	pub length:       usize,
}

/// A layout line for rendering.
#[derive(Debug, Clone)]
pub struct LayoutLine {
	pub text:       String,
	pub has_cursor: bool,
	pub cursor_pos: Option<usize>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_default_state() {
		let state = EditorState::default();
		assert!(state.is_empty());
		assert_eq!(state.text(), "");
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_state_text() {
		let state = EditorState {
			lines:       vec!["hello".into(), "world".into()],
			cursor_line: 1,
			cursor_col:  5,
		};
		assert_eq!(state.text(), "hello\nworld");
		assert!(!state.is_empty());
	}

	#[test]
	fn test_current_line() {
		let state = EditorState {
			lines:       vec!["first".into(), "second".into()],
			cursor_line: 1,
			cursor_col:  3,
		};
		assert_eq!(state.current_line(), "second");
		assert_eq!(state.text_before_cursor(), "sec");
		assert_eq!(state.text_after_cursor(), "ond");
	}
}
