//! Word-wrap layout for the editor.
//!
//! Splits logical lines into visual lines using word-boundary-aware wrapping.
//! Falls back to character-level wrapping for words wider than the available
//! width.

use unicode_segmentation::UnicodeSegmentation;

use super::state::{EditorState, LayoutLine, VisualLine};

/// A chunk of text produced by word wrapping, with position info.
#[derive(Debug, Clone)]
pub struct TextChunk {
	pub text:        String,
	pub start_index: usize,
	pub end_index:   usize,
}

/// Check if a grapheme cluster is whitespace.
pub fn is_whitespace_grapheme(g: &str) -> bool {
	g.chars()
		.next()
		.is_some_and(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n')
}

/// Check if a grapheme cluster is punctuation.
pub fn is_punctuation_grapheme(g: &str) -> bool {
	const PUNCTUATION: &str = "(){}[]<>.,;:'\"!?+-=*/\\|&%^$#@~`";
	g.chars().next().is_some_and(|c| PUNCTUATION.contains(c))
}

/// Word-wrap a single line into chunks that fit within `max_width` columns.
///
/// Wraps at word boundaries when possible, falling back to character-level
/// wrapping for words longer than the available width.
pub fn word_wrap_line(line: &str, max_width: usize) -> Vec<TextChunk> {
	if line.is_empty() || max_width == 0 {
		return vec![TextChunk { text: String::new(), start_index: 0, end_index: 0 }];
	}

	let line_width = rho_text::width::visible_width_str(line);
	if line_width <= max_width {
		return vec![TextChunk {
			text:        line.to_owned(),
			start_index: 0,
			end_index:   line.len(),
		}];
	}

	// Split into tokens (words and whitespace runs)
	let mut tokens: Vec<Token> = Vec::new();
	let mut current_token = String::new();
	let mut token_start: usize = 0;
	let mut in_whitespace = false;
	let mut byte_index: usize = 0;

	for grapheme in line.graphemes(true) {
		let is_ws = is_whitespace_grapheme(grapheme);

		if current_token.is_empty() {
			in_whitespace = is_ws;
			token_start = byte_index;
		} else if is_ws != in_whitespace {
			tokens.push(Token {
				text:          current_token.clone(),
				start_index:   token_start,
				end_index:     byte_index,
				is_whitespace: in_whitespace,
			});
			current_token.clear();
			token_start = byte_index;
			in_whitespace = is_ws;
		}

		current_token.push_str(grapheme);
		byte_index += grapheme.len();
	}
	if !current_token.is_empty() {
		tokens.push(Token {
			text:          current_token,
			start_index:   token_start,
			end_index:     byte_index,
			is_whitespace: in_whitespace,
		});
	}

	// Build chunks using word wrapping
	let mut chunks: Vec<TextChunk> = Vec::new();
	let mut current_chunk = String::new();
	let mut current_width: usize = 0;
	let mut chunk_start_index: usize = 0;
	let mut at_line_start = true;

	for token in &tokens {
		let token_width = rho_text::width::visible_width_str(&token.text);

		// Skip leading whitespace at line start
		if at_line_start && token.is_whitespace {
			chunk_start_index = token.end_index;
			continue;
		}
		at_line_start = false;

		// Token wider than max_width → break by grapheme
		if token_width > max_width {
			// Push accumulated chunk first
			if !current_chunk.is_empty() {
				chunks.push(TextChunk {
					text:        current_chunk.clone(),
					start_index: chunk_start_index,
					end_index:   token.start_index,
				});
				current_chunk.clear();
				current_width = 0;
				chunk_start_index = token.start_index;
			}

			// Break the long token by grapheme
			let mut token_chunk = String::new();
			let mut token_chunk_width: usize = 0;
			let mut token_chunk_start = token.start_index;
			let mut token_byte_index = token.start_index;

			for grapheme in token.text.graphemes(true) {
				let gw = rho_text::width::visible_width_str(grapheme);

				if token_chunk_width + gw > max_width && !token_chunk.is_empty() {
					chunks.push(TextChunk {
						text:        token_chunk.clone(),
						start_index: token_chunk_start,
						end_index:   token_byte_index,
					});
					token_chunk.clear();
					grapheme.clone_into(&mut token_chunk);
					token_chunk_width = gw;
					token_chunk_start = token_byte_index;
				} else {
					token_chunk.push_str(grapheme);
					token_chunk_width += gw;
				}
				token_byte_index += grapheme.len();
			}

			// Keep remainder as start of next chunk
			if !token_chunk.is_empty() {
				current_chunk = token_chunk;
				current_width = token_chunk_width;
				chunk_start_index = token_chunk_start;
			}
			continue;
		}

		// Check if adding this token exceeds width
		if current_width + token_width > max_width {
			// Push current chunk (trim trailing whitespace)
			let trimmed = current_chunk.trim_end().to_owned();
			if !trimmed.is_empty() || chunks.is_empty() {
				chunks.push(TextChunk {
					text:        trimmed,
					start_index: chunk_start_index,
					end_index:   chunk_start_index + current_chunk.len(),
				});
			}

			// Start new line — skip leading whitespace
			at_line_start = true;
			if token.is_whitespace {
				current_chunk.clear();
				current_width = 0;
				chunk_start_index = token.end_index;
			} else {
				current_chunk.clone_from(&token.text);
				current_width = token_width;
				chunk_start_index = token.start_index;
				at_line_start = false;
			}
		} else {
			current_chunk.push_str(&token.text);
			current_width += token_width;
		}
	}

	// Push final chunk
	if !current_chunk.is_empty() {
		chunks.push(TextChunk {
			text:        current_chunk,
			start_index: chunk_start_index,
			end_index:   line.len(),
		});
	}

	if chunks.is_empty() {
		vec![TextChunk { text: String::new(), start_index: 0, end_index: 0 }]
	} else {
		chunks
	}
}

/// Build the layout lines for rendering, applying word wrapping and cursor
/// placement.
pub fn layout_text(state: &EditorState, content_width: usize) -> Vec<LayoutLine> {
	let mut layout_lines = Vec::new();

	if state.is_empty() {
		layout_lines.push(LayoutLine {
			text:       String::new(),
			has_cursor: true,
			cursor_pos: Some(0),
		});
		return layout_lines;
	}

	for (i, line) in state.lines.iter().enumerate() {
		let is_current_line = i == state.cursor_line;
		let line_vis_width = rho_text::width::visible_width_str(line);

		if line_vis_width <= content_width {
			if is_current_line {
				layout_lines.push(LayoutLine {
					text:       line.clone(),
					has_cursor: true,
					cursor_pos: Some(state.cursor_col),
				});
			} else {
				layout_lines.push(LayoutLine {
					text:       line.clone(),
					has_cursor: false,
					cursor_pos: None,
				});
			}
		} else {
			let chunks = word_wrap_line(line, content_width);

			for (chunk_index, chunk) in chunks.iter().enumerate() {
				let is_last_chunk = chunk_index == chunks.len() - 1;

				let (has_cursor, cursor_pos) = if is_current_line {
					let cursor_pos_val = state.cursor_col;
					if is_last_chunk {
						// Last chunk: cursor belongs here if >= start_index
						if cursor_pos_val >= chunk.start_index {
							let adjusted = cursor_pos_val - chunk.start_index;
							(true, Some(adjusted))
						} else {
							(false, None)
						}
					} else {
						// Non-last chunk: cursor belongs here if in [start_index, end_index)
						if cursor_pos_val >= chunk.start_index && cursor_pos_val < chunk.end_index {
							let adjusted = (cursor_pos_val - chunk.start_index).min(chunk.text.len());
							(true, Some(adjusted))
						} else {
							(false, None)
						}
					}
				} else {
					(false, None)
				};

				layout_lines.push(LayoutLine {
					text: chunk.text.clone(),
					has_cursor,
					cursor_pos: if has_cursor { cursor_pos } else { None },
				});
			}
		}
	}

	layout_lines
}

/// Build a mapping from visual lines to logical positions.
pub fn build_visual_line_map(state: &EditorState, width: usize) -> Vec<VisualLine> {
	let mut visual_lines = Vec::new();

	for (i, line) in state.lines.iter().enumerate() {
		if line.is_empty() {
			visual_lines.push(VisualLine { logical_line: i, start_col: 0, length: 0 });
		} else {
			let line_vis_width = rho_text::width::visible_width_str(line);
			if line_vis_width <= width {
				visual_lines.push(VisualLine {
					logical_line: i,
					start_col:    0,
					length:       line.len(),
				});
			} else {
				let chunks = word_wrap_line(line, width);
				for chunk in &chunks {
					visual_lines.push(VisualLine {
						logical_line: i,
						start_col:    chunk.start_index,
						length:       chunk.end_index - chunk.start_index,
					});
				}
			}
		}
	}

	visual_lines
}

/// Find the visual line index for the current cursor position.
pub fn find_current_visual_line(state: &EditorState, visual_lines: &[VisualLine]) -> usize {
	for (i, vl) in visual_lines.iter().enumerate() {
		if vl.logical_line == state.cursor_line {
			let col_in_segment = state.cursor_col as isize - vl.start_col as isize;
			let is_last_segment_of_line = i == visual_lines.len() - 1
				|| visual_lines
					.get(i + 1)
					.is_some_and(|next| next.logical_line != vl.logical_line);

			if col_in_segment >= 0
				&& (col_in_segment < vl.length as isize
					|| (is_last_segment_of_line && col_in_segment <= vl.length as isize))
			{
				return i;
			}
		}
	}
	// Fallback: return last visual line
	visual_lines.len().saturating_sub(1)
}

/// Token for word-wrap tokenization.
struct Token {
	text:          String,
	start_index:   usize,
	end_index:     usize,
	is_whitespace: bool,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_word_wrap_empty() {
		let chunks = word_wrap_line("", 10);
		assert_eq!(chunks.len(), 1);
		assert_eq!(chunks[0].text, "");
	}

	#[test]
	fn test_word_wrap_fits() {
		let chunks = word_wrap_line("hello", 10);
		assert_eq!(chunks.len(), 1);
		assert_eq!(chunks[0].text, "hello");
	}

	#[test]
	fn test_word_wrap_at_boundary() {
		let chunks = word_wrap_line("hello world", 5);
		assert_eq!(chunks.len(), 2);
		assert_eq!(chunks[0].text, "hello");
		assert_eq!(chunks[1].text, "world");
	}

	#[test]
	fn test_word_wrap_long_word() {
		let chunks = word_wrap_line("abcdefghijklmno", 5);
		assert!(chunks.len() >= 3);
		for chunk in &chunks {
			assert!(rho_text::width::visible_width_str(&chunk.text) <= 5);
		}
	}

	#[test]
	fn test_word_wrap_multiple_words() {
		let chunks = word_wrap_line("the quick brown fox", 10);
		for chunk in &chunks {
			assert!(rho_text::width::visible_width_str(&chunk.text) <= 10);
		}
		// Verify all text preserved
		let reconstructed: String = chunks
			.iter()
			.map(|c| c.text.as_str())
			.collect::<Vec<_>>()
			.join(" ");
		// Whitespace may differ but all words should be present
		assert!(reconstructed.contains("the"));
		assert!(reconstructed.contains("quick"));
		assert!(reconstructed.contains("brown"));
		assert!(reconstructed.contains("fox"));
	}

	#[test]
	fn test_layout_empty() {
		let state = EditorState::default();
		let lines = layout_text(&state, 80);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].has_cursor);
		assert_eq!(lines[0].cursor_pos, Some(0));
	}

	#[test]
	fn test_layout_single_line() {
		let state = EditorState { lines: vec!["hello".into()], cursor_line: 0, cursor_col: 3 };
		let lines = layout_text(&state, 80);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].has_cursor);
		assert_eq!(lines[0].cursor_pos, Some(3));
	}

	#[test]
	fn test_layout_multi_line() {
		let state = EditorState {
			lines:       vec!["hello".into(), "world".into()],
			cursor_line: 1,
			cursor_col:  2,
		};
		let lines = layout_text(&state, 80);
		assert_eq!(lines.len(), 2);
		assert!(!lines[0].has_cursor);
		assert!(lines[1].has_cursor);
		assert_eq!(lines[1].cursor_pos, Some(2));
	}

	#[test]
	fn test_visual_line_map_simple() {
		let state = EditorState {
			lines:       vec!["hello".into(), "world".into()],
			cursor_line: 0,
			cursor_col:  0,
		};
		let map = build_visual_line_map(&state, 80);
		assert_eq!(map.len(), 2);
		assert_eq!(map[0].logical_line, 0);
		assert_eq!(map[1].logical_line, 1);
	}

	#[test]
	fn test_visual_line_map_with_wrap() {
		let state =
			EditorState { lines: vec!["hello world".into()], cursor_line: 0, cursor_col: 0 };
		let map = build_visual_line_map(&state, 5);
		assert!(map.len() >= 2);
		assert_eq!(map[0].logical_line, 0);
		assert_eq!(map[1].logical_line, 0);
	}

	#[test]
	fn test_find_current_visual_line() {
		let state = EditorState {
			lines:       vec!["hello".into(), "world".into()],
			cursor_line: 1,
			cursor_col:  3,
		};
		let map = build_visual_line_map(&state, 80);
		let vl = find_current_visual_line(&state, &map);
		assert_eq!(vl, 1);
	}

	#[test]
	fn test_is_whitespace() {
		assert!(is_whitespace_grapheme(" "));
		assert!(is_whitespace_grapheme("\t"));
		assert!(!is_whitespace_grapheme("a"));
		assert!(!is_whitespace_grapheme("!"));
	}

	#[test]
	fn test_is_punctuation() {
		assert!(is_punctuation_grapheme("."));
		assert!(is_punctuation_grapheme("!"));
		assert!(is_punctuation_grapheme("("));
		assert!(!is_punctuation_grapheme("a"));
		assert!(!is_punctuation_grapheme(" "));
	}
}
