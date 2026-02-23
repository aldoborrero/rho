//! ANSI-aware word wrapping.
//!
//! Wraps text to a visible width, preserving ANSI escape codes across line
//! breaks.

use smallvec::{SmallVec, smallvec};

use crate::{
	ansi::{
		AnsiState, ansi_seq_len_bytes, ansi_seq_len_u16, is_sgr_bytes, is_sgr_u16,
		update_state_from_text_bytes, update_state_from_text_u16,
	},
	width::{
		ascii_cell_width_u16, for_each_grapheme_u16_slow, grapheme_width_str, visible_width_str,
		visible_width_u16,
	},
};

// ============================================================================
// UTF-16 wrapping
// ============================================================================

#[inline]
fn write_active_codes_u16(state: &AnsiState, out: &mut Vec<u16>) {
	if !state.is_empty() {
		state.write_restore_u16(out);
	}
}

#[inline]
fn write_line_end_reset_u16(state: &AnsiState, out: &mut Vec<u16>) {
	if state.has_underline() {
		out.extend_from_slice(&[crate::ESC_U16, b'[' as u16, b'2' as u16, b'4' as u16, b'm' as u16]);
	}
}

fn token_is_whitespace_u16(token: &[u16]) -> bool {
	let mut i = 0usize;
	while i < token.len() {
		if token[i] == crate::ESC_U16
			&& let Some(seq_len) = ansi_seq_len_u16(token, i)
		{
			i += seq_len;
			continue;
		}
		if token[i] != b' ' as u16 {
			return false;
		}
		i += 1;
	}
	true
}

fn trim_end_spaces_in_place_u16(line: &mut Vec<u16>) {
	while let Some(&last) = line.last() {
		if last == b' ' as u16 {
			line.pop();
		} else {
			break;
		}
	}
}

fn split_into_tokens_with_ansi_u16(line: &[u16]) -> SmallVec<[Vec<u16>; 4]> {
	let mut tokens = SmallVec::<[Vec<u16>; 4]>::new();
	let mut current = Vec::<u16>::new();
	let mut pending_ansi = SmallVec::<[u16; 32]>::new();
	let mut in_whitespace = false;
	let mut i = 0usize;

	while i < line.len() {
		if line[i] == crate::ESC_U16
			&& let Some(seq_len) = ansi_seq_len_u16(line, i)
		{
			pending_ansi.extend_from_slice(&line[i..i + seq_len]);
			i += seq_len;
			continue;
		}

		let ch = line[i];
		let char_is_space = ch == b' ' as u16;
		if char_is_space != in_whitespace && !current.is_empty() {
			tokens.push(current);
			current = Vec::new();
		}

		if !pending_ansi.is_empty() {
			current.extend_from_slice(&pending_ansi);
			pending_ansi.clear();
		}

		in_whitespace = char_is_space;
		current.push(ch);
		i += 1;
	}

	if !pending_ansi.is_empty() {
		current.extend_from_slice(&pending_ansi);
	}

	if !current.is_empty() {
		tokens.push(current);
	}

	tokens
}

fn break_long_word_u16(
	word: &[u16],
	width: usize,
	state: &mut AnsiState,
) -> SmallVec<[Vec<u16>; 4]> {
	let mut lines = SmallVec::<[Vec<u16>; 4]>::new();
	let mut current_line = Vec::<u16>::new();
	write_active_codes_u16(state, &mut current_line);
	let mut current_width = 0usize;
	let mut i = 0usize;

	while i < word.len() {
		if word[i] == crate::ESC_U16
			&& let Some(seq_len) = ansi_seq_len_u16(word, i)
		{
			let seq = &word[i..i + seq_len];
			current_line.extend_from_slice(seq);
			if is_sgr_u16(seq) {
				state.apply_sgr_u16(&seq[2..seq_len - 1]);
			}
			i += seq_len;
			continue;
		}

		let start = i;
		let mut is_ascii = true;
		while i < word.len() && word[i] != crate::ESC_U16 {
			if word[i] > 0x7f {
				is_ascii = false;
			}
			i += 1;
		}
		let seg = &word[start..i];

		if is_ascii {
			for &u in seg {
				let gw = ascii_cell_width_u16(u);
				if current_width + gw > width {
					write_line_end_reset_u16(state, &mut current_line);
					lines.push(current_line);
					current_line = Vec::new();
					write_active_codes_u16(state, &mut current_line);
					current_width = 0;
				}
				current_line.push(u);
				current_width += gw;
			}
		} else {
			let _ = for_each_grapheme_u16_slow(seg, |gu16, gw| {
				if current_width + gw > width {
					write_line_end_reset_u16(state, &mut current_line);
					lines.push(std::mem::take(&mut current_line));
					write_active_codes_u16(state, &mut current_line);
					current_width = 0;
				}
				current_line.extend_from_slice(gu16);
				current_width += gw;
				true
			});
		}
	}

	if !current_line.is_empty() {
		lines.push(current_line);
	}

	lines
}

fn wrap_single_line_u16(line: &[u16], width: usize) -> SmallVec<[Vec<u16>; 4]> {
	if line.is_empty() {
		return smallvec![Vec::new()];
	}

	if visible_width_u16(line) <= width {
		return smallvec![line.to_vec()];
	}

	let tokens = split_into_tokens_with_ansi_u16(line);
	let mut wrapped = SmallVec::<[Vec<u16>; 4]>::new();
	let mut current_line = Vec::<u16>::new();
	let mut current_width = 0usize;
	let mut state = AnsiState::new();

	for token in tokens {
		let token_width = visible_width_u16(&token);
		let is_whitespace = token_is_whitespace_u16(&token);

		if token_width > width && !is_whitespace {
			if !current_line.is_empty() {
				write_line_end_reset_u16(&state, &mut current_line);
				wrapped.push(current_line);
				current_line = Vec::new();
				current_width = 0;
			}

			let mut broken = break_long_word_u16(&token, width, &mut state);
			if let Some(last) = broken.pop() {
				wrapped.extend(broken);
				current_line = last;
				current_width = visible_width_u16(&current_line);
			}
			continue;
		}

		let total_needed = current_width + token_width;
		if total_needed > width && current_width > 0 {
			let mut line_to_wrap = current_line;
			trim_end_spaces_in_place_u16(&mut line_to_wrap);
			write_line_end_reset_u16(&state, &mut line_to_wrap);
			wrapped.push(line_to_wrap);

			current_line = Vec::new();
			write_active_codes_u16(&state, &mut current_line);
			if is_whitespace {
				current_width = 0;
			} else {
				current_line.extend_from_slice(&token);
				current_width = token_width;
			}
		} else {
			current_line.extend_from_slice(&token);
			current_width += token_width;
		}

		update_state_from_text_u16(&token, &mut state);
	}

	if !current_line.is_empty() {
		wrapped.push(current_line);
	}

	for line in &mut wrapped {
		trim_end_spaces_in_place_u16(line);
	}

	if wrapped.is_empty() {
		wrapped.push(Vec::new());
	}

	wrapped
}

/// Wrap UTF-16 text to a visible width, preserving ANSI escape codes.
pub fn wrap_text_with_ansi_u16(text: &[u16], width: usize) -> SmallVec<[Vec<u16>; 4]> {
	if text.is_empty() {
		return smallvec![Vec::new()];
	}

	let mut result = SmallVec::<[Vec<u16>; 4]>::new();
	let mut state = AnsiState::new();
	let mut line_start = 0usize;

	for i in 0..=text.len() {
		if i == text.len() || text[i] == b'\n' as u16 {
			let line = &text[line_start..i];
			let mut line_with_prefix: Vec<u16> = Vec::new();
			if !result.is_empty() {
				write_active_codes_u16(&state, &mut line_with_prefix);
			}
			line_with_prefix.extend_from_slice(line);

			let wrapped = wrap_single_line_u16(&line_with_prefix, width);
			result.extend(wrapped);
			update_state_from_text_u16(line, &mut state);
			line_start = i + 1;
		}
	}

	if result.is_empty() {
		result.push(Vec::new());
	}

	result
}

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

	fn to_u16(s: &str) -> Vec<u16> {
		s.encode_utf16().collect()
	}

	#[test]
	fn test_wrap_basic_u16() {
		let data = to_u16("hello world");
		let lines = wrap_text_with_ansi_u16(&data, 5);
		assert_eq!(lines.len(), 2);
		assert_eq!(String::from_utf16_lossy(&lines[0]), "hello");
		assert_eq!(String::from_utf16_lossy(&lines[1]), "world");
	}

	#[test]
	fn test_wrap_basic_str() {
		let lines = wrap_text_with_ansi_str("hello world", 5);
		assert_eq!(lines.len(), 2);
		assert_eq!(lines[0], "hello");
		assert_eq!(lines[1], "world");
	}

	#[test]
	fn test_wrap_preserves_color_u16() {
		let data = to_u16("\x1b[38;2;156;163;176mhello world\x1b[0m");
		let lines = wrap_text_with_ansi_u16(&data, 5);
		assert_eq!(lines.len(), 2);
		let first = String::from_utf16_lossy(&lines[0]);
		let second = String::from_utf16_lossy(&lines[1]);
		assert!(first.starts_with("\x1b[38;2;156;163;176m"));
		assert!(second.starts_with("\x1b[38;2;156;163;176m"));
		assert!(second.contains("world"));
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
