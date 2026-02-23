//! Visible width calculation for ANSI-escaped text.
//!
//! Measures terminal cell width, skipping escape sequences.
//! Supports both UTF-16 and UTF-8 text.

use std::cell::RefCell;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
	TAB_WIDTH,
	ansi::{ansi_seq_len_bytes, ansi_seq_len_u16},
};

// ============================================================================
// Cell width helpers
// ============================================================================

#[inline]
pub(crate) const fn ascii_cell_width_u16(u: u16) -> usize {
	let b = u as u8;
	match b {
		b'\t' => TAB_WIDTH,
		0x20..=0x7e => 1,
		_ => 0,
	}
}

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
// Grapheme iteration (UTF-16 slow path)
// ============================================================================

thread_local! {
  pub(crate) static SCRATCH: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Iterate graphemes in a non-ASCII UTF-16 segment.
///
/// Callback returns `true` to continue, `false` to stop early.
#[inline]
pub(crate) fn for_each_grapheme_u16_slow<F>(segment: &[u16], mut f: F) -> bool
where
	F: FnMut(&[u16], usize) -> bool,
{
	if segment.is_empty() {
		return true;
	}

	SCRATCH.with_borrow_mut(|scratch| {
		scratch.clear();
		scratch.reserve(segment.len());

		for r in std::char::decode_utf16(segment.iter().copied()) {
			scratch.push(r.unwrap_or('\u{FFFD}'));
		}

		let mut utf16_pos = 0usize;
		for g in scratch.graphemes(true) {
			let w = grapheme_width_str(g);

			let g_u16_len: usize = g.chars().map(|c| c.len_utf16()).sum();
			let u16_slice = &segment[utf16_pos..utf16_pos + g_u16_len];
			utf16_pos += g_u16_len;

			if !f(u16_slice, w) {
				return false;
			}
		}

		true
	})
}

// ============================================================================
// UTF-16 visible width
// ============================================================================

/// Visible width with early-exit if width exceeds `limit`.
pub(crate) fn visible_width_u16_up_to(data: &[u16], limit: usize) -> (usize, bool) {
	let mut width = 0usize;
	let mut i = 0usize;
	let len = data.len();

	while i < len {
		if data[i] == crate::ESC_U16 {
			if let Some(seq_len) = ansi_seq_len_u16(data, i) {
				i += seq_len;
				continue;
			}
			i += 1;
			continue;
		}

		let start = i;
		let mut is_ascii = true;
		while i < len && data[i] != crate::ESC_U16 {
			if data[i] > 0x7f {
				is_ascii = false;
			}
			i += 1;
		}
		let seg = &data[start..i];

		if is_ascii {
			for &u in seg {
				width += ascii_cell_width_u16(u);
				if width > limit {
					return (width, true);
				}
			}
		} else {
			let ok = for_each_grapheme_u16_slow(seg, |_, w| {
				width += w;
				width <= limit
			});
			if !ok {
				return (width, true);
			}
		}
	}

	(width, width > limit)
}

/// Calculate visible width of UTF-16 text, excluding ANSI escape sequences.
pub fn visible_width_u16(data: &[u16]) -> usize {
	visible_width_u16_up_to(data, usize::MAX).0
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

	fn to_u16(s: &str) -> Vec<u16> {
		s.encode_utf16().collect()
	}

	#[test]
	fn test_visible_width_u16() {
		assert_eq!(visible_width_u16(&to_u16("hello")), 5);
		assert_eq!(visible_width_u16(&to_u16("\x1b[31mhello\x1b[0m")), 5);
		assert_eq!(visible_width_u16(&to_u16("\x1b[38;5;196mred\x1b[0m")), 3);
		assert_eq!(visible_width_u16(&to_u16("a\tb")), 1 + TAB_WIDTH + 1);
	}

	#[test]
	fn test_visible_width_str() {
		assert_eq!(visible_width_str("hello"), 5);
		assert_eq!(visible_width_str("\x1b[31mhello\x1b[0m"), 5);
		assert_eq!(visible_width_str("\x1b[38;5;196mred\x1b[0m"), 3);
		assert_eq!(visible_width_str("a\tb"), 1 + TAB_WIDTH + 1);
	}

	#[test]
	fn test_early_exit() {
		let data = to_u16(&"a]b".repeat(1000));
		let (w, exceeded) = visible_width_u16_up_to(&data, 10);
		assert!(exceeded);
		assert!(w > 10);
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
		assert_eq!(visible_width_u16(&to_u16("世界")), 4);
	}
}
