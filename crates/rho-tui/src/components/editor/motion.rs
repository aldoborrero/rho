//! Cursor movement for the editor.
//!
//! Handles vertical movement with sticky column, horizontal grapheme-aware
//! movement, word boundary navigation, and line start/end.

use unicode_segmentation::UnicodeSegmentation;

use super::{
	layout::{
		build_visual_line_map, find_current_visual_line, is_punctuation_grapheme,
		is_whitespace_grapheme,
	},
	state::{EditorState, VisualLine},
};

/// Move the cursor vertically and/or horizontally.
///
/// `delta_line`: -1 for up, +1 for down.
/// `delta_col`: -1 for left, +1 for right.
///
/// Returns the new `preferred_visual_col` value.
pub fn move_cursor(
	state: &mut EditorState,
	delta_line: i32,
	delta_col: i32,
	last_layout_width: usize,
	preferred_visual_col: &mut Option<usize>,
) {
	let visual_lines = build_visual_line_map(state, last_layout_width);
	let current_visual_line = find_current_visual_line(state, &visual_lines);

	if delta_line != 0 {
		let target = current_visual_line as i32 + delta_line;
		if target >= 0 && (target as usize) < visual_lines.len() {
			move_to_visual_line(
				state,
				&visual_lines,
				current_visual_line,
				target as usize,
				preferred_visual_col,
			);
		}
	}

	if delta_col != 0 {
		let current_line = state.current_line().to_owned();

		if delta_col > 0 {
			// Moving right — by one grapheme
			if state.cursor_col < current_line.len() {
				let after = &current_line[state.cursor_col..];
				let first_grapheme = after.graphemes(true).next();
				let advance = first_grapheme.map_or(1, str::len);
				set_cursor_col(state, state.cursor_col + advance, preferred_visual_col);
			} else if state.cursor_line < state.lines.len() - 1 {
				// Wrap to start of next logical line
				state.cursor_line += 1;
				set_cursor_col(state, 0, preferred_visual_col);
			} else {
				// At end of last line — set preferred_visual_col for up/down navigation
				if let Some(current_vl) = visual_lines.get(current_visual_line) {
					*preferred_visual_col = Some(state.cursor_col - current_vl.start_col);
				}
			}
		} else {
			// Moving left — by one grapheme
			if state.cursor_col > 0 {
				let before = &current_line[..state.cursor_col];
				let last_grapheme = before.graphemes(true).next_back();
				let retreat = last_grapheme.map_or(1, str::len);
				set_cursor_col(state, state.cursor_col - retreat, preferred_visual_col);
			} else if state.cursor_line > 0 {
				// Wrap to end of previous logical line
				state.cursor_line -= 1;
				let prev_line = state.current_line().to_owned();
				set_cursor_col(state, prev_line.len(), preferred_visual_col);
			}
		}
	}
}

/// Move to a target visual line, applying sticky column logic.
fn move_to_visual_line(
	state: &mut EditorState,
	visual_lines: &[VisualLine],
	current_visual_line: usize,
	target_visual_line: usize,
	preferred_visual_col: &mut Option<usize>,
) {
	let Some(current_vl) = visual_lines.get(current_visual_line) else {
		return;
	};
	let Some(target_vl) = visual_lines.get(target_visual_line) else {
		return;
	};

	let current_visual_col = state.cursor_col.saturating_sub(current_vl.start_col);

	// Compute max visual column for source segment
	let is_last_source_segment = current_visual_line == visual_lines.len() - 1
		|| visual_lines
			.get(current_visual_line + 1)
			.is_some_and(|next| next.logical_line != current_vl.logical_line);
	let source_max = if is_last_source_segment {
		current_vl.length
	} else {
		current_vl.length.saturating_sub(1)
	};

	// Compute max visual column for target segment
	let is_last_target_segment = target_visual_line == visual_lines.len() - 1
		|| visual_lines
			.get(target_visual_line + 1)
			.is_some_and(|next| next.logical_line != target_vl.logical_line);
	let target_max = if is_last_target_segment {
		target_vl.length
	} else {
		target_vl.length.saturating_sub(1)
	};

	let move_to = compute_vertical_move_column(
		current_visual_col,
		source_max,
		target_max,
		preferred_visual_col,
	);

	state.cursor_line = target_vl.logical_line;
	let target_col = target_vl.start_col + move_to;
	let logical_line_len = state
		.lines
		.get(target_vl.logical_line)
		.map_or(0, String::len);
	state.cursor_col = target_col.min(logical_line_len);
}

/// Compute the target visual column for vertical movement.
/// Implements the sticky column decision table.
const fn compute_vertical_move_column(
	current_visual_col: usize,
	source_max: usize,
	target_max: usize,
	preferred: &mut Option<usize>,
) -> usize {
	let has_preferred = preferred.is_some();
	let cursor_in_middle = current_visual_col < source_max;
	let target_too_short = target_max < current_visual_col;

	if !has_preferred || cursor_in_middle {
		if target_too_short {
			*preferred = Some(current_visual_col);
			return target_max;
		}
		*preferred = None;
		return current_visual_col;
	}

	let pref_val = preferred.unwrap();
	let target_cant_fit = target_max < pref_val;
	if target_too_short || target_cant_fit {
		return target_max;
	}

	let result = pref_val;
	*preferred = None;
	result
}

/// Set cursor column and clear preferred visual col (non-vertical movement).
pub const fn set_cursor_col(
	state: &mut EditorState,
	col: usize,
	preferred_visual_col: &mut Option<usize>,
) {
	state.cursor_col = col;
	*preferred_visual_col = None;
}

/// Move cursor to start of current logical line.
pub const fn move_to_line_start(state: &mut EditorState, preferred_visual_col: &mut Option<usize>) {
	set_cursor_col(state, 0, preferred_visual_col);
}

/// Move cursor to end of current logical line.
pub fn move_to_line_end(state: &mut EditorState, preferred_visual_col: &mut Option<usize>) {
	let len = state.current_line().len();
	set_cursor_col(state, len, preferred_visual_col);
}

/// Check if cursor is on the first visual line.
pub fn is_on_first_visual_line(state: &EditorState, layout_width: usize) -> bool {
	let visual_lines = build_visual_line_map(state, layout_width);
	let current = find_current_visual_line(state, &visual_lines);
	current == 0
}

/// Check if cursor is on the last visual line.
pub fn is_on_last_visual_line(state: &EditorState, layout_width: usize) -> bool {
	let visual_lines = build_visual_line_map(state, layout_width);
	let current = find_current_visual_line(state, &visual_lines);
	current == visual_lines.len() - 1
}

/// Move cursor backward by one word.
/// Skips trailing whitespace, then skips a punctuation run or word run.
pub fn move_word_backwards(state: &mut EditorState, preferred_visual_col: &mut Option<usize>) {
	let current_line = state.current_line().to_owned();

	// At start of line → move to end of previous line
	if state.cursor_col == 0 {
		if state.cursor_line > 0 {
			state.cursor_line -= 1;
			let prev_line = state.current_line().to_owned();
			set_cursor_col(state, prev_line.len(), preferred_visual_col);
		}
		return;
	}

	let before = &current_line[..state.cursor_col];
	let graphemes: Vec<&str> = before.graphemes(true).collect();
	let mut new_col = state.cursor_col;
	let mut idx = graphemes.len();

	// Skip trailing whitespace
	while idx > 0 && is_whitespace_grapheme(graphemes[idx - 1]) {
		new_col -= graphemes[idx - 1].len();
		idx -= 1;
	}

	if idx > 0 {
		let last = graphemes[idx - 1];
		if is_punctuation_grapheme(last) {
			// Skip punctuation run
			while idx > 0 && is_punctuation_grapheme(graphemes[idx - 1]) {
				new_col -= graphemes[idx - 1].len();
				idx -= 1;
			}
		} else {
			// Skip word run
			while idx > 0
				&& !is_whitespace_grapheme(graphemes[idx - 1])
				&& !is_punctuation_grapheme(graphemes[idx - 1])
			{
				new_col -= graphemes[idx - 1].len();
				idx -= 1;
			}
		}
	}

	set_cursor_col(state, new_col, preferred_visual_col);
}

/// Move cursor forward by one word.
/// Skips leading whitespace, then skips a punctuation run or word run.
pub fn move_word_forwards(state: &mut EditorState, preferred_visual_col: &mut Option<usize>) {
	let current_line = state.current_line().to_owned();

	// At end of line → move to start of next line
	if state.cursor_col >= current_line.len() {
		if state.cursor_line < state.lines.len() - 1 {
			state.cursor_line += 1;
			set_cursor_col(state, 0, preferred_visual_col);
		}
		return;
	}

	let after = &current_line[state.cursor_col..];
	let mut grapheme_iter = after.graphemes(true).peekable();
	let mut new_col = state.cursor_col;

	// Skip leading whitespace
	while grapheme_iter
		.peek()
		.is_some_and(|g| is_whitespace_grapheme(g))
	{
		new_col += grapheme_iter.next().unwrap().len();
	}

	if let Some(&first) = grapheme_iter.peek() {
		if is_punctuation_grapheme(first) {
			// Skip punctuation run
			while grapheme_iter
				.peek()
				.is_some_and(|g| is_punctuation_grapheme(g))
			{
				new_col += grapheme_iter.next().unwrap().len();
			}
		} else {
			// Skip word run
			while grapheme_iter
				.peek()
				.is_some_and(|g| !is_whitespace_grapheme(g) && !is_punctuation_grapheme(g))
			{
				new_col += grapheme_iter.next().unwrap().len();
			}
		}
	}

	set_cursor_col(state, new_col, preferred_visual_col);
}

/// Jump to the first occurrence of `target` in the specified direction.
/// Multi-line search, case-sensitive, skips current cursor position.
pub fn jump_to_char(
	state: &mut EditorState,
	target: &str,
	forward: bool,
	preferred_visual_col: &mut Option<usize>,
) {
	if forward {
		// Search forward
		for line_idx in state.cursor_line..state.lines.len() {
			let line = &state.lines[line_idx];
			let search_from = if line_idx == state.cursor_line {
				state.cursor_col + 1
			} else {
				0
			};
			if search_from <= line.len()
				&& let Some(idx) = line[search_from..].find(target)
			{
				state.cursor_line = line_idx;
				set_cursor_col(state, search_from + idx, preferred_visual_col);
				return;
			}
		}
	} else {
		// Search backward
		for line_idx in (0..=state.cursor_line).rev() {
			let line = &state.lines[line_idx];
			let search_until = if line_idx == state.cursor_line {
				state.cursor_col.saturating_sub(1)
			} else {
				line.len()
			};
			if let Some(idx) = line[..search_until.min(line.len())].rfind(target) {
				state.cursor_line = line_idx;
				set_cursor_col(state, idx, preferred_visual_col);
				return;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_state(text: &str) -> EditorState {
		let lines: Vec<String> = text.split('\n').map(String::from).collect();
		let cursor_line = lines.len() - 1;
		let cursor_col = lines.last().map_or(0, String::len);
		EditorState { lines, cursor_line, cursor_col }
	}

	#[test]
	fn test_move_right_grapheme() {
		let mut state =
			EditorState { lines: vec!["hello".into()], cursor_line: 0, cursor_col: 0 };
		let mut pref = None;
		move_cursor(&mut state, 0, 1, 80, &mut pref);
		assert_eq!(state.cursor_col, 1);
	}

	#[test]
	fn test_move_left_grapheme() {
		let mut state =
			EditorState { lines: vec!["hello".into()], cursor_line: 0, cursor_col: 3 };
		let mut pref = None;
		move_cursor(&mut state, 0, -1, 80, &mut pref);
		assert_eq!(state.cursor_col, 2);
	}

	#[test]
	fn test_move_right_wraps_line() {
		let mut state = EditorState {
			lines:       vec!["ab".into(), "cd".into()],
			cursor_line: 0,
			cursor_col:  2,
		};
		let mut pref = None;
		move_cursor(&mut state, 0, 1, 80, &mut pref);
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_move_left_wraps_line() {
		let mut state = EditorState {
			lines:       vec!["ab".into(), "cd".into()],
			cursor_line: 1,
			cursor_col:  0,
		};
		let mut pref = None;
		move_cursor(&mut state, 0, -1, 80, &mut pref);
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 2);
	}

	#[test]
	fn test_move_up_down_simple() {
		let mut state = EditorState {
			lines:       vec!["hello".into(), "world".into()],
			cursor_line: 0,
			cursor_col:  3,
		};
		let mut pref = None;
		// Down
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 3);
		// Up
		move_cursor(&mut state, -1, 0, 80, &mut pref);
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 3);
	}

	#[test]
	fn test_sticky_column_through_short_line() {
		let mut state = make_state("2222222222x222\n\n1111111111_111111111111");
		// Go to line 2, col 10
		state.cursor_line = 2;
		state.cursor_col = 10;
		let mut pref = None;

		// Up to empty line → col 0 (clamped)
		move_cursor(&mut state, -1, 0, 80, &mut pref);
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 0);
		assert!(pref.is_some()); // Sticky should be set

		// Up to line 0 → col 10 (restored)
		move_cursor(&mut state, -1, 0, 80, &mut pref);
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 10);
		assert!(pref.is_none()); // Sticky cleared after restoring
	}

	#[test]
	fn test_sticky_column_reset_on_horizontal() {
		let mut state = make_state("1234567890\n\n1234567890");
		state.cursor_line = 2;
		state.cursor_col = 5;
		let mut pref = None;

		// Up through empty line
		move_cursor(&mut state, -1, 0, 80, &mut pref);
		move_cursor(&mut state, -1, 0, 80, &mut pref);
		assert_eq!(state.cursor_col, 5);

		// Left resets sticky
		move_cursor(&mut state, 0, -1, 80, &mut pref);
		assert_eq!(state.cursor_col, 4);
		assert!(pref.is_none());

		// Down twice — uses 4 (not 5)
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		assert_eq!(state.cursor_col, 4);
	}

	#[test]
	fn test_move_word_backwards() {
		let mut state =
			EditorState { lines: vec!["foo bar baz".into()], cursor_line: 0, cursor_col: 11 };
		let mut pref = None;

		move_word_backwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 8); // before "baz"

		move_word_backwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 4); // before "bar"

		move_word_backwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 0); // before "foo"
	}

	#[test]
	fn test_move_word_forwards() {
		let mut state =
			EditorState { lines: vec!["foo bar baz".into()], cursor_line: 0, cursor_col: 0 };
		let mut pref = None;

		move_word_forwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 3); // after "foo"

		move_word_forwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 7); // after "bar"

		move_word_forwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 11); // after "baz"
	}

	#[test]
	fn test_move_word_backwards_with_punctuation() {
		let mut state =
			EditorState { lines: vec!["foo bar...".into()], cursor_line: 0, cursor_col: 10 };
		let mut pref = None;

		move_word_backwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 7); // before "..."

		move_word_backwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 4); // before "bar"
	}

	#[test]
	fn test_move_word_forwards_with_punctuation() {
		let mut state =
			EditorState { lines: vec!["foo bar... baz".into()], cursor_line: 0, cursor_col: 4 };
		let mut pref = None;

		move_word_forwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 7); // after "bar"

		move_word_forwards(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 10); // after "..."
	}

	#[test]
	fn test_move_word_backwards_line_boundary() {
		let mut state = EditorState {
			lines:       vec!["foo".into(), "bar".into()],
			cursor_line: 1,
			cursor_col:  0,
		};
		let mut pref = None;

		move_word_backwards(&mut state, &mut pref);
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 3);
	}

	#[test]
	fn test_move_word_forwards_line_boundary() {
		let mut state = EditorState {
			lines:       vec!["foo".into(), "bar".into()],
			cursor_line: 0,
			cursor_col:  3,
		};
		let mut pref = None;

		move_word_forwards(&mut state, &mut pref);
		assert_eq!(state.cursor_line, 1);
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_jump_to_char_forward() {
		let mut state =
			EditorState { lines: vec!["hello world".into()], cursor_line: 0, cursor_col: 0 };
		let mut pref = None;

		jump_to_char(&mut state, "w", true, &mut pref);
		assert_eq!(state.cursor_col, 6);
	}

	#[test]
	fn test_jump_to_char_backward() {
		let mut state =
			EditorState { lines: vec!["hello world".into()], cursor_line: 0, cursor_col: 10 };
		let mut pref = None;

		jump_to_char(&mut state, "h", false, &mut pref);
		assert_eq!(state.cursor_col, 0);
	}

	#[test]
	fn test_move_to_line_start_end() {
		let mut state =
			EditorState { lines: vec!["hello".into()], cursor_line: 0, cursor_col: 3 };
		let mut pref = None;

		move_to_line_start(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 0);

		move_to_line_end(&mut state, &mut pref);
		assert_eq!(state.cursor_col, 5);
	}

	#[test]
	fn test_multiple_consecutive_up_down() {
		let mut state = make_state("1234567890\nab\ncd\nef\n1234567890");
		// Start at line 4, col 7
		state.cursor_line = 4;
		state.cursor_col = 7;
		let mut pref = None;

		// Move up multiple times through short lines
		move_cursor(&mut state, -1, 0, 80, &mut pref); // line 3, col 2 (clamped)
		move_cursor(&mut state, -1, 0, 80, &mut pref); // line 2, col 2 (clamped)
		move_cursor(&mut state, -1, 0, 80, &mut pref); // line 1, col 2 (clamped)
		move_cursor(&mut state, -1, 0, 80, &mut pref); // line 0, col 7 (restored)
		assert_eq!(state.cursor_line, 0);
		assert_eq!(state.cursor_col, 7);

		// Move down back through short lines
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		move_cursor(&mut state, 1, 0, 80, &mut pref);
		assert_eq!(state.cursor_line, 4);
		assert_eq!(state.cursor_col, 7);
	}
}
