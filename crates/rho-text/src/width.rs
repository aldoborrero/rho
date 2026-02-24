//! Visible width calculation for ANSI-escaped text.
//!
//! Measures terminal cell width, skipping escape sequences.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{TAB_WIDTH, ansi::ansi_seq_len_bytes};

// ============================================================================
// Cell width helpers
// ============================================================================

#[inline]
pub(crate) fn grapheme_width_str(g: &str) -> usize {
	if g == "\t" {
		return TAB_WIDTH;
	}
	let mut it = g.chars();
	let Some(c0) = it.next() else {
		return 0;
	};
	if it.next().is_none() {
		return UnicodeWidthChar::width(c0).unwrap_or(0);
	}
	UnicodeWidthStr::width(g)
}

// ============================================================================
// UTF-8 visible width
// ============================================================================

/// Visible width of UTF-8 text with early-exit.
pub(crate) fn visible_width_str_up_to(data: &str, limit: usize) -> (usize, bool) {
	let bytes = data.as_bytes();
	let mut width = 0usize;
	let mut byte_pos = 0usize;
	let len = bytes.len();

	while byte_pos < len {
		if bytes[byte_pos] == crate::ESC_U8 {
			if let Some(seq_len) = ansi_seq_len_bytes(bytes, byte_pos) {
				byte_pos += seq_len;
				continue;
			}
			byte_pos += 1;
			continue;
		}

		// Find the next non-ANSI segment
		let start = byte_pos;
		while byte_pos < len && bytes[byte_pos] != crate::ESC_U8 {
			byte_pos += 1;
		}
		let seg = &data[start..byte_pos];

		for g in seg.graphemes(true) {
			width += grapheme_width_str(g);
			if width > limit {
				return (width, true);
			}
		}
	}

	(width, width > limit)
}

/// Calculate visible width of UTF-8 text, excluding ANSI escape sequences.
pub fn visible_width_str(data: &str) -> usize {
	visible_width_str_up_to(data, usize::MAX).0
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_visible_width_str() {
		assert_eq!(visible_width_str("hello"), 5);
		assert_eq!(visible_width_str("\x1b[31mhello\x1b[0m"), 5);
		assert_eq!(visible_width_str("\x1b[38;5;196mred\x1b[0m"), 3);
		assert_eq!(visible_width_str("a\tb"), 1 + TAB_WIDTH + 1);
	}

	#[test]
	fn test_early_exit_str() {
		let data = "a]b".repeat(1000);
		let (w, exceeded) = visible_width_str_up_to(&data, 10);
		assert!(exceeded);
		assert!(w > 10);
	}

	#[test]
	fn test_wide_chars() {
		// CJK characters are 2 cells wide
		assert_eq!(visible_width_str("世界"), 4);
	}
}
