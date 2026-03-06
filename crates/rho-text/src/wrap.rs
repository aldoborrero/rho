//! ANSI-aware word wrapping.
//!
//! Wraps text to a visible width, preserving ANSI escape codes across line
//! breaks.

use smallvec::{SmallVec, smallvec};

use crate::{
	ansi::{AnsiState, ansi_seq_len_bytes, is_sgr_bytes, update_state_from_text_bytes},
	width::{grapheme_width_str, visible_width_str},
};

// ============================================================================
// UTF-8 wrapping
// ============================================================================

#[inline]
fn write_active_codes_str(state: &AnsiState, out: &mut String) {
	if !state.is_empty() {
		state.write_restore_str(out);
	}
}

#[inline]
fn write_line_end_reset_str(state: &AnsiState, out: &mut String) {
	if state.has_underline() {
		out.push_str("\x1b[24m");
	}
}

fn token_is_whitespace_str(token: &str) -> bool {
	let bytes = token.as_bytes();
	let mut i = 0usize;
	while i < bytes.len() {
		if bytes[i] == crate::ESC_U8
			&& let Some(seq_len) = ansi_seq_len_bytes(bytes, i)
		{
			i += seq_len;
			continue;
		}
		if bytes[i] != b' ' {
			return false;
		}
		i += 1;
	}
	true
}

fn split_into_tokens_with_ansi_str(line: &str) -> SmallVec<[String; 4]> {
	let bytes = line.as_bytes();
	let mut tokens = SmallVec::<[String; 4]>::new();
	let mut current = String::new();
	let mut pending_ansi = String::new();
	let mut in_whitespace = false;
	let mut i = 0usize;

	while i < bytes.len() {
		if bytes[i] == crate::ESC_U8
			&& let Some(seq_len) = ansi_seq_len_bytes(bytes, i)
		{
			pending_ansi.push_str(&line[i..i + seq_len]);
			i += seq_len;
			continue;
		}

		let ch = bytes[i];
		let char_is_space = ch == b' ';
		if char_is_space != in_whitespace && !current.is_empty() {
			tokens.push(current);
			current = String::new();
		}

		if !pending_ansi.is_empty() {
			current.push_str(&pending_ansi);
			pending_ansi.clear();
		}

		in_whitespace = char_is_space;

		// Handle multi-byte UTF-8 characters
		let char_len = utf8_char_len(bytes[i]);
		current.push_str(&line[i..i + char_len]);
		i += char_len;
	}

	if !pending_ansi.is_empty() {
		current.push_str(&pending_ansi);
	}

	if !current.is_empty() {
		tokens.push(current);
	}

	tokens
}

#[inline]
const fn utf8_char_len(first_byte: u8) -> usize {
	match first_byte {
		0..=0x7f => 1,
		0xc0..=0xdf => 2,
		0xe0..=0xef => 3,
		0xf0..=0xf7 => 4,
		_ => 1,
	}
}

fn break_long_word_str(word: &str, width: usize, state: &mut AnsiState) -> SmallVec<[String; 4]> {
	let bytes = word.as_bytes();
	let mut lines = SmallVec::<[String; 4]>::new();
	let mut current_line = String::new();
	write_active_codes_str(state, &mut current_line);
	let mut current_width = 0usize;
	let mut i = 0usize;

	while i < bytes.len() {
		if bytes[i] == crate::ESC_U8
			&& let Some(seq_len) = ansi_seq_len_bytes(bytes, i)
		{
			let seq = &word[i..i + seq_len];
			current_line.push_str(seq);
			if is_sgr_bytes(&bytes[i..i + seq_len]) {
				state.apply_sgr_bytes(&bytes[i + 2..i + seq_len - 1]);
			}
			i += seq_len;
			continue;
		}

		let start = i;
		while i < bytes.len() && bytes[i] != crate::ESC_U8 {
			i += 1;
		}
		let seg = &word[start..i];

		use unicode_segmentation::UnicodeSegmentation;
		for g in seg.graphemes(true) {
			let gw = grapheme_width_str(g);
			if current_width + gw > width {
				write_line_end_reset_str(state, &mut current_line);
				lines.push(current_line);
				current_line = String::new();
				write_active_codes_str(state, &mut current_line);
				current_width = 0;
			}
			current_line.push_str(g);
			current_width += gw;
		}
	}

	if !current_line.is_empty() {
		lines.push(current_line);
	}

	lines
}

fn wrap_single_line_str(line: &str, width: usize) -> SmallVec<[String; 4]> {
	if line.is_empty() {
		return smallvec![String::new()];
	}

	if visible_width_str(line) <= width {
		return smallvec![line.to_owned()];
	}

	let tokens = split_into_tokens_with_ansi_str(line);
	let mut wrapped = SmallVec::<[String; 4]>::new();
	let mut current_line = String::new();
	let mut current_width = 0usize;
	let mut state = AnsiState::new();

	for token in &tokens {
		let token_width = visible_width_str(token);
		let is_whitespace = token_is_whitespace_str(token);

		if token_width > width && !is_whitespace {
			if !current_line.is_empty() {
				write_line_end_reset_str(&state, &mut current_line);
				wrapped.push(current_line);
				current_line = String::new();
				current_width = 0;
			}

			let mut broken = break_long_word_str(token, width, &mut state);
			if let Some(last) = broken.pop() {
				wrapped.extend(broken);
				current_line = last;
				current_width = visible_width_str(&current_line);
			}
			continue;
		}

		let total_needed = current_width + token_width;
		if total_needed > width && current_width > 0 {
			// Trim trailing spaces
			while current_line.ends_with(' ') {
				current_line.pop();
			}
			write_line_end_reset_str(&state, &mut current_line);
			wrapped.push(current_line);

			current_line = String::new();
			write_active_codes_str(&state, &mut current_line);
			if is_whitespace {
				current_width = 0;
			} else {
				current_line.push_str(token);
				current_width = token_width;
			}
		} else {
			current_line.push_str(token);
			current_width += token_width;
		}

		update_state_from_text_bytes(token.as_bytes(), &mut state);
	}

	if !current_line.is_empty() {
		wrapped.push(current_line);
	}

	for line in &mut wrapped {
		while line.ends_with(' ') {
			line.pop();
		}
	}

	if wrapped.is_empty() {
		wrapped.push(String::new());
	}

	wrapped
}

/// Wrap UTF-8 text to a visible width, preserving ANSI escape codes.
pub fn wrap_text_with_ansi_str(text: &str, width: usize) -> SmallVec<[String; 4]> {
	if text.is_empty() {
		return smallvec![String::new()];
	}

	let mut result = SmallVec::<[String; 4]>::new();
	let mut state = AnsiState::new();

	for (idx, line) in text.split('\n').enumerate() {
		let mut line_with_prefix = String::new();
		if idx > 0 {
			write_active_codes_str(&state, &mut line_with_prefix);
		}
		line_with_prefix.push_str(line);

		let wrapped = wrap_single_line_str(&line_with_prefix, width);
		result.extend(wrapped);
		update_state_from_text_bytes(line.as_bytes(), &mut state);
	}

	if result.is_empty() {
		result.push(String::new());
	}

	result
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_wrap_basic_str() {
		let lines = wrap_text_with_ansi_str("hello world", 5);
		assert_eq!(lines.len(), 2);
		assert_eq!(lines[0], "hello");
		assert_eq!(lines[1], "world");
	}

	#[test]
	fn test_wrap_preserves_color_str() {
		let lines = wrap_text_with_ansi_str("\x1b[38;2;156;163;176mhello world\x1b[0m", 5);
		assert_eq!(lines.len(), 2);
		assert!(lines[0].starts_with("\x1b[38;2;156;163;176m"));
		assert!(lines[1].starts_with("\x1b[38;2;156;163;176m"));
		assert!(lines[1].contains("world"));
	}

	#[test]
	fn test_wrap_empty() {
		let lines = wrap_text_with_ansi_str("", 80);
		assert_eq!(lines.len(), 1);
		assert_eq!(lines[0], "");
	}

	#[test]
	fn test_wrap_newlines() {
		let lines = wrap_text_with_ansi_str("hello\nworld", 80);
		assert_eq!(lines.len(), 2);
		assert_eq!(lines[0], "hello");
		assert_eq!(lines[1], "world");
	}

	#[test]
	fn test_wrap_fits() {
		let lines = wrap_text_with_ansi_str("hello", 80);
		assert_eq!(lines.len(), 1);
		assert_eq!(lines[0], "hello");
	}
}
