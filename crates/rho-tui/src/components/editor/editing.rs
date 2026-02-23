//! Text editing operations for the editor.
//!
//! Handles insert, delete, backspace, line splitting/merging — all
//! grapheme-aware.

use unicode_segmentation::UnicodeSegmentation;

use super::state::EditorState;

/// Insert a single character or grapheme at the cursor position.
pub fn insert_character(state: &mut EditorState, ch: &str) {
	let line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();
	let before = &line[..state.cursor_col.min(line.len())];
	let after = &line[state.cursor_col.min(line.len())..];
	state.lines[state.cursor_line] = format!("{before}{ch}{after}");
	state.cursor_col += ch.len();
}

/// Insert a multi-line text at the cursor position.
pub fn insert_text_at_cursor(state: &mut EditorState, text: &str) {
	let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
	let lines: Vec<&str> = normalized.split('\n').collect();

	if lines.len() == 1 {
		let line = state
			.lines
			.get(state.cursor_line)
			.cloned()
			.unwrap_or_default();
		let before = &line[..state.cursor_col.min(line.len())];
		let after = &line[state.cursor_col.min(line.len())..];
		state.lines[state.cursor_line] = format!("{before}{normalized}{after}");
		state.cursor_col += normalized.len();
	} else {
		let current_line = state
			.lines
			.get(state.cursor_line)
			.cloned()
			.unwrap_or_default();
		let before_cursor = &current_line[..state.cursor_col.min(current_line.len())];
		let after_cursor = &current_line[state.cursor_col.min(current_line.len())..];

		let mut new_lines: Vec<String> = Vec::new();

		// Lines before current
		for i in 0..state.cursor_line {
			new_lines.push(state.lines.get(i).cloned().unwrap_or_default());
		}

		// First inserted line joins with text before cursor
		new_lines.push(format!("{before_cursor}{}", lines[0]));

		// Middle inserted lines
		for item in lines.iter().skip(1).take(lines.len().saturating_sub(2)) {
			new_lines.push((*item).to_owned());
		}

		// Last inserted line joins with text after cursor
		new_lines.push(format!("{}{after_cursor}", lines.last().unwrap_or(&"")));

		// Lines after current
		for i in (state.cursor_line + 1)..state.lines.len() {
			new_lines.push(state.lines.get(i).cloned().unwrap_or_default());
		}

		state.lines = new_lines;
		state.cursor_line += lines.len() - 1;
		state.cursor_col = lines.last().map_or(0, |l| l.len());
	}
}

/// Handle backspace at cursor position.
/// Returns true if text was modified.
pub fn handle_backspace(state: &mut EditorState) -> bool {
	if state.cursor_col > 0 {
		let line = state
			.lines
			.get(state.cursor_line)
			.cloned()
			.unwrap_or_default();
		let before_cursor = &line[..state.cursor_col];

		// Find last grapheme
		let graphemes: Vec<&str> = before_cursor.graphemes(true).collect();
		let last_grapheme = graphemes.last().copied().unwrap_or("");
		let grapheme_len = last_grapheme.len();

		let before = &line[..state.cursor_col - grapheme_len];
		let after = &line[state.cursor_col..];
		state.lines[state.cursor_line] = format!("{before}{after}");
		state.cursor_col -= grapheme_len;
		true
	} else if state.cursor_line > 0 {
		// Merge with previous line
		let current = state.lines.remove(state.cursor_line);
		state.cursor_line -= 1;
		let prev_len = state.lines[state.cursor_line].len();
		state.lines[state.cursor_line].push_str(&current);
		state.cursor_col = prev_len;
		true
	} else {
		false
	}
}

/// Handle forward delete at cursor position.
/// Returns true if text was modified.
pub fn handle_forward_delete(state: &mut EditorState) -> bool {
	let current_line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();

	if state.cursor_col < current_line.len() {
		let after_cursor = &current_line[state.cursor_col..];

		// Find first grapheme at cursor
		let first_grapheme = after_cursor.graphemes(true).next().unwrap_or("");
		let grapheme_len = first_grapheme.len();

		let before = &current_line[..state.cursor_col];
		let after = &current_line[state.cursor_col + grapheme_len..];
		state.lines[state.cursor_line] = format!("{before}{after}");
		true
	} else if state.cursor_line < state.lines.len() - 1 {
		// Merge with next line
		let next = state.lines.remove(state.cursor_line + 1);
		state.lines[state.cursor_line].push_str(&next);
		true
	} else {
		false
	}
}

/// Add a new line at cursor position (split current line).
pub fn add_new_line(state: &mut EditorState) {
	let current_line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();
	let before = current_line[..state.cursor_col.min(current_line.len())].to_owned();
	let after = current_line[state.cursor_col.min(current_line.len())..].to_owned();

	state.lines[state.cursor_line] = before;
	state.lines.insert(state.cursor_line + 1, after);
	state.cursor_line += 1;
	state.cursor_col = 0;
}

/// Delete from cursor to end of line (Ctrl+K).
/// Returns the deleted text.
pub fn delete_to_end_of_line(state: &mut EditorState) -> String {
	let current_line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();

	if state.cursor_col < current_line.len() {
		let deleted = current_line[state.cursor_col..].to_owned();
		current_line[..state.cursor_col].clone_into(&mut state.lines[state.cursor_line]);
		deleted
	} else if state.cursor_line < state.lines.len() - 1 {
		// At end of line — merge with next line
		let next = state.lines.remove(state.cursor_line + 1);
		state.lines[state.cursor_line].push_str(&next);
		"\n".to_owned()
	} else {
		String::new()
	}
}

/// Delete from cursor to start of line (Ctrl+U).
/// Returns the deleted text.
pub fn delete_to_start_of_line(state: &mut EditorState) -> String {
	let current_line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();

	if state.cursor_col > 0 {
		let deleted = current_line[..state.cursor_col].to_owned();
		current_line[state.cursor_col..].clone_into(&mut state.lines[state.cursor_line]);
		state.cursor_col = 0;
		deleted
	} else if state.cursor_line > 0 {
		// At start of line — merge with previous line
		let current = state.lines.remove(state.cursor_line);
		state.cursor_line -= 1;
		let prev_len = state.lines[state.cursor_line].len();
		state.lines[state.cursor_line].push_str(&current);
		state.cursor_col = prev_len;
		"\n".to_owned()
	} else {
		String::new()
	}
}

/// Delete word backwards (Ctrl+W / Alt+Backspace).
/// Returns the deleted text.
pub fn delete_word_backwards(state: &mut EditorState) -> String {
	let current_line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();

	if state.cursor_col == 0 {
		if state.cursor_line > 0 {
			// Merge with previous line
			let current = state.lines.remove(state.cursor_line);
			state.cursor_line -= 1;
			let prev_len = state.lines[state.cursor_line].len();
			state.lines[state.cursor_line].push_str(&current);
			state.cursor_col = prev_len;
			return "\n".to_owned();
		}
		return String::new();
	}

	// Save position, move word backwards to find delete boundary
	let old_col = state.cursor_col;
	let mut temp_pref = None;
	super::motion::move_word_backwards(state, &mut temp_pref);
	let delete_from = state.cursor_col;
	state.cursor_col = old_col; // restore to get the deleted text

	let deleted = current_line[delete_from..old_col].to_owned();
	state.lines[state.cursor_line] =
		format!("{}{}", &current_line[..delete_from], &current_line[old_col..]);
	state.cursor_col = delete_from;

	deleted
}

/// Delete word forwards (Alt+D / Alt+Delete).
/// Returns the deleted text.
pub fn delete_word_forwards(state: &mut EditorState) -> String {
	let current_line = state
		.lines
		.get(state.cursor_line)
		.cloned()
		.unwrap_or_default();

	if state.cursor_col >= current_line.len() {
		if state.cursor_line < state.lines.len() - 1 {
			// Merge with next line
			let next = state.lines.remove(state.cursor_line + 1);
			state.lines[state.cursor_line].push_str(&next);
			return "\n".to_owned();
		}
		return String::new();
	}

	// Save position, move word forwards to find delete boundary
	let old_col = state.cursor_col;
	let mut temp_pref = None;
	super::motion::move_word_forwards(state, &mut temp_pref);
	let delete_to = state.cursor_col;
	state.cursor_col = old_col; // restore

	let deleted = current_line[old_col..delete_to].to_owned();
	state.lines[state.cursor_line] =
		format!("{}{}", &current_line[..old_col], &current_line[delete_to..]);

	deleted
}

/// Delete the most recently yanked text from the buffer (for yank-pop).
/// Assumes cursor is at the end of the yanked text.
/// Returns true if deletion succeeded.
pub fn delete_yanked_text(state: &mut EditorState, yanked_text: &str) -> bool {
	let yank_lines: Vec<&str> = yanked_text.split('\n').collect();
	let end_line = state.cursor_line;
	let end_col = state.cursor_col;
	let start_line = end_line.checked_sub(yank_lines.len() - 1);
	let Some(start_line) = start_line else {
		return false;
	};

	if yank_lines.len() == 1 {
		let line = state.lines.get(end_line).cloned().unwrap_or_default();
		let start_col = end_col.checked_sub(yanked_text.len());
		let Some(start_col) = start_col else {
			return false;
		};
		if line.get(start_col..end_col) != Some(yanked_text) {
			return false;
		}
		state.lines[end_line] = format!("{}{}", &line[..start_col], &line[end_col..]);
		state.cursor_line = end_line;
		state.cursor_col = start_col;
		return true;
	}

	let first_inserted = yank_lines[0];
	let last_inserted = yank_lines.last().unwrap_or(&"");
	let first_line_text = state.lines.get(start_line).cloned().unwrap_or_default();
	let last_line_text = state.lines.get(end_line).cloned().unwrap_or_default();

	if !first_line_text.ends_with(first_inserted) {
		return false;
	}
	if end_col != last_inserted.len() {
		return false;
	}
	if last_line_text.get(..end_col) != Some(last_inserted) {
		return false;
	}

	let start_col = first_line_text.len() - first_inserted.len();
	let suffix = &last_line_text[end_col..];
	let new_line = format!("{}{suffix}", &first_line_text[..start_col]);

	// Remove the range [start_line..=end_line] and replace with new_line
	state
		.lines
		.splice(start_line..=end_line, std::iter::once(new_line));
	state.cursor_line = start_line;
	state.cursor_col = start_col;
	true
}

/// Set text, splitting by newlines, placing cursor at end.
pub fn set_text_internal(state: &mut EditorState, text: &str) {
	let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
	let lines: Vec<String> = normalized.split('\n').map(String::from).collect();
	state.lines = if lines.is_empty() {
		vec![String::new()]
	} else {
		lines
	};
	state.cursor_line = state.lines.len() - 1;
	state.cursor_col = state.lines[state.cursor_line].len();
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_state(text: &str) -> EditorState {
		let mut state = EditorState::default();
		set_text_internal(&mut state, text);
		state
	}

	#[test]
	fn test_insert_character() {
		let mut state =
			EditorState { lines: vec!["abc".into()], cursor_line: 0, cursor_col: 1 };
		insert_character(&mut state, "X");
		assert_eq!(state.lines[0], "aXbc");
		assert_eq!(state.cursor_col, 2);
	}

	#[test]
	fn test_insert_emoji() {
		let mut state =
			EditorState { lines: vec!["ab".into()], cursor_line: 0, cursor_col: 1 };
		insert_character(&mut state, "😀");
		assert_eq!(state.lines[0], "a😀b");
		assert_eq!(state.cursor_col, 1 + "😀".len());
	}

	#[test]
	fn test_backspace_basic() {
		let mut state =
			EditorState { lines: vec!["abc".into()], cursor_line: 0, cursor_col: 2 };
		assert!(handle_backspace(&mut state));
		assert_eq!(state.lines[0], "ac");
		assert_eq!(state.cursor_col, 1);
	}

	#[test]
	fn test_backspace_emoji() {
		let mut state = make_state("😀👍");
		// cursor at end
		assert!(handle_backspace(&mut state));
		assert_eq!(state.lines[0], "😀");
	}

	#[test]
	fn test_backspace_merges_lines() {
		let mut state = EditorState {
			lines:       vec!["abc".into(), "def".into()],
			cursor_line: 1,
			cursor_col:  0,
		};
		assert!(handle_backspace(&mut state));
		assert_eq!(state.lines, vec!["abcdef"]);
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 3);
	}

	#[test]
	fn test_forward_delete() {
		let mut state =
			EditorState { lines: vec!["abc".into()], cursor_line: 0, cursor_col: 1 };
		assert!(handle_forward_delete(&mut state));
		assert_eq!(state.lines[0], "ac");
		assert_eq!(state.cursor_col, 1);
	}

	#[test]
	fn test_forward_delete_merges_lines() {
		let mut state = EditorState {
			lines:       vec!["abc".into(), "def".into()],
			cursor_line: 0,
			cursor_col:  3,
		};
		assert!(handle_forward_delete(&mut state));
		assert_eq!(state.lines, vec!["abcdef"]);
	}

	#[test]
	fn test_add_new_line() {
		let mut state =
			EditorState { lines: vec!["abcdef".into()], cursor_line: 0, cursor_col: 3 };
		add_new_line(&mut state);
		assert_eq!(state.lines, vec!["abc", "def"]);
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_delete_to_end_of_line() {
		let mut state =
			EditorState { lines: vec!["hello world".into()], cursor_line: 0, cursor_col: 5 };
		let deleted = delete_to_end_of_line(&mut state);
		assert_eq!(deleted, " world");
		assert_eq!(state.lines[0], "hello");
	}

	#[test]
	fn test_delete_to_start_of_line() {
		let mut state =
			EditorState { lines: vec!["hello world".into()], cursor_line: 0, cursor_col: 5 };
		let deleted = delete_to_start_of_line(&mut state);
		assert_eq!(deleted, "hello");
		assert_eq!(state.lines[0], " world");
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_delete_word_backwards() {
		let mut state = make_state("foo bar baz");
		let deleted = delete_word_backwards(&mut state);
		assert_eq!(deleted, "baz");
		assert_eq!(state.text(), "foo bar ");
	}

	#[test]
	fn test_delete_word_backwards_trailing_space() {
		let mut state = make_state("foo bar   ");
		let deleted = delete_word_backwards(&mut state);
		assert_eq!(deleted, "bar   ");
		assert_eq!(state.text(), "foo ");
	}

	#[test]
	fn test_delete_word_forwards() {
		let mut state =
			EditorState { lines: vec!["foo bar baz".into()], cursor_line: 0, cursor_col: 0 };
		let deleted = delete_word_forwards(&mut state);
		assert_eq!(deleted, "foo");
		assert_eq!(state.text(), " bar baz");
	}

	#[test]
	fn test_insert_text_at_cursor_single_line() {
		let mut state =
			EditorState { lines: vec!["ac".into()], cursor_line: 0, cursor_col: 1 };
		insert_text_at_cursor(&mut state, "b");
		assert_eq!(state.text(), "abc");
		assert_eq!(state.cursor_col, 2);
	}

	#[test]
	fn test_insert_text_at_cursor_multi_line() {
		let mut state =
			EditorState { lines: vec!["ac".into()], cursor_line: 0, cursor_col: 1 };
		insert_text_at_cursor(&mut state, "X\nY");
		assert_eq!(state.text(), "aX\nYc");
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 1);
	}

	#[test]
	fn test_set_text_internal() {
		let mut state = EditorState::default();
		set_text_internal(&mut state, "hello\nworld");
		assert_eq!(state.lines, vec!["hello", "world"]);
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 5);
	}

	#[test]
	fn test_set_text_internal_empty() {
		let mut state = EditorState::default();
		set_text_internal(&mut state, "");
		assert_eq!(state.lines, vec![""]);
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_delete_yanked_text_single_line() {
		let mut state =
			EditorState { lines: vec!["hello world".into()], cursor_line: 0, cursor_col: 11 };
		assert!(delete_yanked_text(&mut state, "world"));
		assert_eq!(state.text(), "hello ");
		assert_eq!(state.cursor_col, 6);
	}

	#[test]
	fn test_delete_yanked_text_multi_line() {
		let mut state = EditorState {
			lines:       vec!["aX".into(), "Yc".into()],
			cursor_line: 1,
			cursor_col:  1,
		};
		assert!(delete_yanked_text(&mut state, "X\nY"));
		assert_eq!(state.text(), "ac");
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 1);
	}
}
