//! Truncate text to a visible width with optional ellipsis.

use crate::{
	ansi::{ansi_seq_len_bytes, is_sgr_bytes},
	width::{grapheme_width_str, visible_width_str_up_to},
};

/// Kind of ellipsis to append when truncating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EllipsisKind {
	/// Unicode ellipsis character `…` (1 cell wide)
	Unicode = 0,
	/// ASCII ellipsis `...` (3 cells wide)
	Ascii   = 1,
	/// No ellipsis
	None    = 2,
}

impl From<u8> for EllipsisKind {
	fn from(v: u8) -> Self {
		match v {
			0 => Self::Unicode,
			1 => Self::Ascii,
			2 => Self::None,
			_ => Self::Unicode,
		}
	}
}

// ============================================================================
// UTF-8 truncation
// ============================================================================

/// Truncate UTF-8 text to a visible width with optional ellipsis.
///
/// Returns `None` if the text already fits (caller can use original).
/// Returns `Some(truncated)` if truncation was needed.
pub fn truncate_to_width_str(
	text: &str,
	max_width: usize,
	ellipsis_kind: EllipsisKind,
	pad: bool,
) -> Option<String> {
	let (text_w, exceeded) = visible_width_str_up_to(text, max_width);
	if !exceeded {
		if !pad || text_w == max_width {
			return None;
		}
		let mut out = String::with_capacity(text.len() + (max_width - text_w));
		out.push_str(text);
		for _ in 0..(max_width - text_w) {
			out.push(' ');
		}
		return Some(out);
	}

	let (ellipsis, ellipsis_w): (&str, usize) = match ellipsis_kind {
		EllipsisKind::Unicode => ("\u{2026}", 1),
		EllipsisKind::Ascii => ("...", 3),
		EllipsisKind::None => ("", 0),
	};

	let target_w = max_width.saturating_sub(ellipsis_w);

	if target_w == 0 {
		let mut out = String::with_capacity(ellipsis.len() + max_width);
		// Ellipsis alone doesn't fit — take what we can
		use unicode_segmentation::UnicodeSegmentation;
		let mut w = 0usize;
		for g in ellipsis.graphemes(true) {
			let gw = grapheme_width_str(g);
			if w + gw > max_width {
				break;
			}
			out.push_str(g);
			w += gw;
		}
		if pad && w < max_width {
			for _ in 0..(max_width - w) {
				out.push(' ');
			}
		}
		return Some(out);
	}

	let bytes = text.as_bytes();
	let mut out = String::with_capacity(text.len().min(max_width * 4) + ellipsis.len() + 8);
	let mut w = 0usize;
	let mut byte_pos = 0usize;
	let mut saw_sgr = false;

	while byte_pos < bytes.len() {
		if bytes[byte_pos] == crate::ESC_U8 {
			if let Some(seq_len) = ansi_seq_len_bytes(bytes, byte_pos) {
				let seq = &text[byte_pos..byte_pos + seq_len];
				out.push_str(seq);
				if is_sgr_bytes(&bytes[byte_pos..byte_pos + seq_len]) {
					saw_sgr = true;
				}
				byte_pos += seq_len;
				continue;
			}
			out.push('\x1b');
			byte_pos += 1;
			continue;
		}

		let start = byte_pos;
		while byte_pos < bytes.len() && bytes[byte_pos] != crate::ESC_U8 {
			byte_pos += 1;
		}
		let seg = &text[start..byte_pos];

		use unicode_segmentation::UnicodeSegmentation;
		let mut done = false;
		for g in seg.graphemes(true) {
			let gw = grapheme_width_str(g);
			if w + gw > target_w {
				done = true;
				break;
			}
			out.push_str(g);
			w += gw;
		}
		if done {
			break;
		}
	}

	if saw_sgr {
		out.push_str("\x1b[0m");
	}
	out.push_str(ellipsis);

	if pad {
		let out_w = w + ellipsis_w;
		if out_w < max_width {
			for _ in 0..(max_width - out_w) {
				out.push(' ');
			}
		}
	}

	Some(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_truncate_fits() {
		assert!(truncate_to_width_str("hello", 10, EllipsisKind::Unicode, false).is_none());
	}

	#[test]
	fn test_truncate_with_ellipsis() {
		let result = truncate_to_width_str("hello world", 8, EllipsisKind::Unicode, false);
		assert_eq!(result.as_deref(), Some("hello w\u{2026}"));
	}

	#[test]
	fn test_truncate_with_ascii_ellipsis() {
		let result = truncate_to_width_str("hello world", 8, EllipsisKind::Ascii, false);
		assert_eq!(result.as_deref(), Some("hello..."));
	}

	#[test]
	fn test_truncate_with_pad() {
		let result = truncate_to_width_str("hi", 5, EllipsisKind::Unicode, true);
		assert_eq!(result.as_deref(), Some("hi   "));
	}

	#[test]
	fn test_truncate_ansi() {
		let result =
			truncate_to_width_str("\x1b[31mhello world\x1b[0m", 8, EllipsisKind::Unicode, false);
		let r = result.unwrap();
		assert!(r.contains("\x1b[31m"));
		assert!(r.contains("\x1b[0m"));
		assert!(r.ends_with('\u{2026}'));
	}
}
