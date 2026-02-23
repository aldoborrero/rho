//! Truncate text to a visible width with optional ellipsis.

use crate::{
	ansi::{ansi_seq_len_bytes, ansi_seq_len_u16, is_sgr_bytes, is_sgr_u16},
	width::{
		ascii_cell_width_u16, for_each_grapheme_u16_slow, grapheme_width_str,
		visible_width_str_up_to, visible_width_u16_up_to,
	},
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
// UTF-16 truncation
// ============================================================================

/// Truncate UTF-16 text to a visible width with optional ellipsis.
///
/// Returns `None` if the text already fits (caller can use original).
/// Returns `Some(truncated)` if truncation was needed.
pub fn truncate_to_width_u16(
	text: &[u16],
	max_width: usize,
	ellipsis_kind: EllipsisKind,
	pad: bool,
) -> Option<Vec<u16>> {
	let (text_w, exceeded) = visible_width_u16_up_to(text, max_width);
	if !exceeded {
		if !pad || text_w == max_width {
			return None; // fits, no change needed
		}
		// Pad with spaces
		let mut out = Vec::with_capacity(text.len() + (max_width - text_w));
		out.extend_from_slice(text);
		out.resize(out.len() + (max_width - text_w), b' ' as u16);
		return Some(out);
	}

	let (ellipsis, ellipsis_w): (&[u16], usize) = match ellipsis_kind {
		EllipsisKind::Unicode => (&[0x2026], 1),
		EllipsisKind::Ascii => (&[0x2e, 0x2e, 0x2e], 3),
		EllipsisKind::None => (&[], 0),
	};

	let target_w = max_width.saturating_sub(ellipsis_w);

	if target_w == 0 {
		let mut out = Vec::with_capacity(ellipsis.len().min(max_width * 2));
		let mut w = 0usize;
		let _ = for_each_grapheme_u16_slow(ellipsis, |gu16, gw| {
			if w + gw > max_width {
				return false;
			}
			out.extend_from_slice(gu16);
			w += gw;
			true
		});
		if pad && w < max_width {
			out.resize(out.len() + (max_width - w), b' ' as u16);
		}
		return Some(out);
	}

	let mut out = Vec::with_capacity(text.len().min(max_width * 2) + ellipsis.len() + 8);
	let mut w = 0usize;
	let mut i = 0usize;
	let text_len = text.len();
	let mut saw_sgr = false;

	while i < text_len {
		if text[i] == crate::ESC_U16 {
			if let Some(seq_len) = ansi_seq_len_u16(text, i) {
				let seq = &text[i..i + seq_len];
				out.extend_from_slice(seq);
				if is_sgr_u16(seq) {
					saw_sgr = true;
				}
				i += seq_len;
				continue;
			}
			out.push(crate::ESC_U16);
			i += 1;
			continue;
		}

		let start = i;
		let mut is_ascii = true;
		while i < text_len && text[i] != crate::ESC_U16 {
			if text[i] > 0x7f {
				is_ascii = false;
			}
			i += 1;
		}
		let seg = &text[start..i];

		if is_ascii {
			for &u in seg {
				let gw = ascii_cell_width_u16(u);
				if w + gw > target_w {
					break;
				}
				out.push(u);
				w += gw;
			}
			if w >= target_w {
				break;
			}
		} else {
			let keep_going = for_each_grapheme_u16_slow(seg, |gu16, gw| {
				if w + gw > target_w {
					return false;
				}
				out.extend_from_slice(gu16);
				w += gw;
				true
			});
			if !keep_going {
				break;
			}
		}
	}

	if saw_sgr {
		out.extend_from_slice(&[crate::ESC_U16, b'[' as u16, b'0' as u16, b'm' as u16]);
	}
	out.extend_from_slice(ellipsis);

	if pad {
		let out_w = w + ellipsis_w;
		if out_w < max_width {
			out.resize(out.len() + (max_width - out_w), b' ' as u16);
		}
	}

	Some(out)
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

	fn to_u16(s: &str) -> Vec<u16> {
		s.encode_utf16().collect()
	}

	#[test]
	fn test_truncate_fits() {
		assert!(truncate_to_width_str("hello", 10, EllipsisKind::Unicode, false).is_none());
		assert!(truncate_to_width_u16(&to_u16("hello"), 10, EllipsisKind::Unicode, false).is_none());
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
