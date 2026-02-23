//! Column-based string slicing with ANSI awareness.
//!
//! `slice_with_width` extracts a range of visible columns from a line.
//! `extract_segments` splits a line around an overlay region, preserving ANSI
//! state.

use smallvec::SmallVec;

use crate::{
	ansi::{AnsiState, ansi_seq_len_bytes, ansi_seq_len_u16, is_sgr_bytes, is_sgr_u16},
	width::{ascii_cell_width_u16, for_each_grapheme_u16_slow, grapheme_width_str},
};

// ============================================================================
// Result types
// ============================================================================

/// Result of a `slice_with_width` call.
pub struct SliceResult<T> {
	/// The sliced text.
	pub text:  T,
	/// Visible width of the slice in terminal cells.
	pub width: usize,
}

/// Result of an `extract_segments` call.
pub struct ExtractSegmentsResult<T> {
	/// Content before the overlay region.
	pub before:       T,
	/// Visible width of the `before` segment.
	pub before_width: usize,
	/// Content after the overlay region.
	pub after:        T,
	/// Visible width of the `after` segment.
	pub after_width:  usize,
}

// ============================================================================
// sliceWithWidth — UTF-16
// ============================================================================

/// Slice a range of visible columns from a UTF-16 line.
pub fn slice_with_width_u16(
	line: &[u16],
	start_col: usize,
	length: usize,
	strict: bool,
) -> SliceResult<Vec<u16>> {
	let (text, width) = slice_with_width_u16_impl(line, start_col, length, strict);
	SliceResult { text, width }
}

pub(crate) fn slice_with_width_u16_impl(
	line: &[u16],
	start_col: usize,
	length: usize,
	strict: bool,
) -> (Vec<u16>, usize) {
	let end_col = start_col.saturating_add(length);

	let mut out = Vec::with_capacity(length * 2);
	let mut out_w = 0usize;

	let mut current_col = 0usize;
	let mut i = 0usize;
	let line_len = line.len();

	let mut pending_ansi: SmallVec<[(usize, usize); 4]> = SmallVec::new();

	while i < line_len && current_col < end_col {
		if line[i] == crate::ESC_U16 {
			if let Some(seq_len) = ansi_seq_len_u16(line, i) {
				if current_col >= start_col {
					out.extend_from_slice(&line[i..i + seq_len]);
				} else {
					pending_ansi.push((i, seq_len));
				}
				i += seq_len;
				continue;
			}
			if current_col >= start_col {
				out.push(crate::ESC_U16);
			}
			i += 1;
			continue;
		}

		let start = i;
		let mut is_ascii = true;
		while i < line_len && line[i] != crate::ESC_U16 {
			if line[i] > 0x7f {
				is_ascii = false;
			}
			i += 1;
		}
		let seg = &line[start..i];

		if is_ascii {
			for &u in seg {
				if current_col >= end_col {
					break;
				}
				let gw = ascii_cell_width_u16(u);
				let in_range = current_col >= start_col;
				let fits = !strict || current_col + gw <= end_col;

				if in_range && fits {
					if !pending_ansi.is_empty() {
						for &(p, l) in &pending_ansi {
							out.extend_from_slice(&line[p..p + l]);
						}
						pending_ansi.clear();
					}
					out.push(u);
					out_w += gw;
				}
				current_col += gw;
			}
		} else {
			let _ = for_each_grapheme_u16_slow(seg, |gu16, gw| {
				if current_col >= end_col {
					return false;
				}

				let in_range = current_col >= start_col;
				let fits = !strict || current_col + gw <= end_col;

				if in_range && fits {
					if !pending_ansi.is_empty() {
						for &(p, l) in &pending_ansi {
							out.extend_from_slice(&line[p..p + l]);
						}
						pending_ansi.clear();
					}
					out.extend_from_slice(gu16);
					out_w += gw;
				}

				current_col += gw;
				current_col < end_col
			});
		}
	}

	// Include trailing ANSI sequences
	while i < line.len() {
		if line[i] == crate::ESC_U16
			&& let Some(len) = ansi_seq_len_u16(line, i)
		{
			out.extend_from_slice(&line[i..i + len]);
			i += len;
			continue;
		}
		break;
	}

	(out, out_w)
}

// ============================================================================
// sliceWithWidth — UTF-8
// ============================================================================

/// Slice a range of visible columns from a UTF-8 line.
pub fn slice_with_width_str(
	line: &str,
	start_col: usize,
	length: usize,
	strict: bool,
) -> SliceResult<String> {
	let bytes = line.as_bytes();
	let end_col = start_col.saturating_add(length);

	let mut out = String::with_capacity(length * 2);
	let mut out_w = 0usize;

	let mut current_col = 0usize;
	let mut byte_pos = 0usize;
	let line_len = bytes.len();

	// Pending ANSI sequences (byte ranges)
	let mut pending_ansi: SmallVec<[(usize, usize); 4]> = SmallVec::new();

	while byte_pos < line_len && current_col < end_col {
		if bytes[byte_pos] == crate::ESC_U8 {
			if let Some(seq_len) = ansi_seq_len_bytes(bytes, byte_pos) {
				if current_col >= start_col {
					out.push_str(&line[byte_pos..byte_pos + seq_len]);
				} else {
					pending_ansi.push((byte_pos, seq_len));
				}
				byte_pos += seq_len;
				continue;
			}
			if current_col >= start_col {
				out.push('\x1b');
			}
			byte_pos += 1;
			continue;
		}

		let start = byte_pos;
		while byte_pos < line_len && bytes[byte_pos] != crate::ESC_U8 {
			byte_pos += 1;
		}
		let seg = &line[start..byte_pos];

		use unicode_segmentation::UnicodeSegmentation;
		for g in seg.graphemes(true) {
			if current_col >= end_col {
				break;
			}
			let gw = grapheme_width_str(g);
			let in_range = current_col >= start_col;
			let fits = !strict || current_col + gw <= end_col;

			if in_range && fits {
				if !pending_ansi.is_empty() {
					for &(p, l) in &pending_ansi {
						out.push_str(&line[p..p + l]);
					}
					pending_ansi.clear();
				}
				out.push_str(g);
				out_w += gw;
			}
			current_col += gw;
		}
	}

	// Include trailing ANSI sequences
	while byte_pos < bytes.len() {
		if bytes[byte_pos] == crate::ESC_U8
			&& let Some(len) = ansi_seq_len_bytes(bytes, byte_pos)
		{
			out.push_str(&line[byte_pos..byte_pos + len]);
			byte_pos += len;
			continue;
		}
		break;
	}

	SliceResult { text: out, width: out_w }
}

// ============================================================================
// extractSegments — UTF-16
// ============================================================================

/// Extract the before/after slices around an overlay region (UTF-16).
///
/// Preserves ANSI state so the `after` segment renders correctly.
pub fn extract_segments_u16(
	line: &[u16],
	before_end: usize,
	after_start: usize,
	after_len: usize,
	strict_after: bool,
) -> ExtractSegmentsResult<Vec<u16>> {
	let (before, bw, after, aw) =
		extract_segments_u16_impl(line, before_end, after_start, after_len, strict_after);
	ExtractSegmentsResult { before, before_width: bw, after, after_width: aw }
}

pub(crate) fn extract_segments_u16_impl(
	line: &[u16],
	before_end: usize,
	after_start: usize,
	after_len: usize,
	strict_after: bool,
) -> (Vec<u16>, usize, Vec<u16>, usize) {
	let after_end = after_start.saturating_add(after_len);

	let mut before = Vec::with_capacity(before_end * 2);
	let mut before_w = 0usize;

	let mut after = Vec::with_capacity(after_len * 2);
	let mut after_w = 0usize;

	let mut current_col = 0usize;
	let mut i = 0usize;
	let line_len = line.len();

	let mut pending_before_ansi: SmallVec<[(usize, usize); 4]> = SmallVec::new();

	let mut after_started = false;
	let mut state = AnsiState::new();

	let done_col = if after_len == 0 {
		before_end
	} else {
		after_end
	};

	while i < line_len && current_col < done_col {
		if line[i] == crate::ESC_U16 {
			if let Some(seq_len) = ansi_seq_len_u16(line, i) {
				let seq = &line[i..i + seq_len];
				if is_sgr_u16(seq) {
					state.apply_sgr_u16(&seq[2..seq_len - 1]);
				}

				if current_col < before_end {
					pending_before_ansi.push((i, seq_len));
				} else if current_col >= after_start && current_col < after_end && after_started {
					after.extend_from_slice(seq);
				}

				i += seq_len;
				continue;
			}

			if current_col < before_end {
				before.push(crate::ESC_U16);
			} else if current_col >= after_start && current_col < after_end && after_started {
				after.push(crate::ESC_U16);
			}
			i += 1;
			continue;
		}

		let start = i;
		let mut is_ascii = true;
		while i < line_len && line[i] != crate::ESC_U16 {
			if line[i] > 0x7f {
				is_ascii = false;
			}
			i += 1;
		}
		let seg = &line[start..i];

		if is_ascii {
			for &u in seg {
				if current_col >= done_col {
					break;
				}
				let gw = ascii_cell_width_u16(u);

				if current_col < before_end {
					if !pending_before_ansi.is_empty() {
						for &(p, l) in &pending_before_ansi {
							before.extend_from_slice(&line[p..p + l]);
						}
						pending_before_ansi.clear();
					}
					before.push(u);
					before_w += gw;
				} else if current_col >= after_start && current_col < after_end {
					let fits = !strict_after || current_col + gw <= after_end;
					if fits {
						if !after_started {
							state.write_restore_u16(&mut after);
							after_started = true;
						}
						after.push(u);
						after_w += gw;
					}
				}
				current_col += gw;
			}
		} else {
			let _ = for_each_grapheme_u16_slow(seg, |gu16, gw| {
				if current_col >= done_col {
					return false;
				}

				if current_col < before_end {
					if !pending_before_ansi.is_empty() {
						for &(p, l) in &pending_before_ansi {
							before.extend_from_slice(&line[p..p + l]);
						}
						pending_before_ansi.clear();
					}
					before.extend_from_slice(gu16);
					before_w += gw;
				} else if current_col >= after_start && current_col < after_end {
					let fits = !strict_after || current_col + gw <= after_end;
					if fits {
						if !after_started {
							state.write_restore_u16(&mut after);
							after_started = true;
						}
						after.extend_from_slice(gu16);
						after_w += gw;
					}
				}

				current_col += gw;
				true
			});
		}
	}

	(before, before_w, after, after_w)
}

// ============================================================================
// extractSegments — UTF-8
// ============================================================================

/// Extract the before/after slices around an overlay region (UTF-8).
pub fn extract_segments_str(
	line: &str,
	before_end: usize,
	after_start: usize,
	after_len: usize,
	strict_after: bool,
) -> ExtractSegmentsResult<String> {
	let bytes = line.as_bytes();
	let after_end = after_start.saturating_add(after_len);

	let mut before = String::with_capacity(before_end * 2);
	let mut before_w = 0usize;

	let mut after = String::with_capacity(after_len * 2);
	let mut after_w = 0usize;

	let mut current_col = 0usize;
	let mut byte_pos = 0usize;
	let line_len = bytes.len();

	let mut pending_before_ansi: SmallVec<[(usize, usize); 4]> = SmallVec::new();

	let mut after_started = false;
	let mut state = AnsiState::new();

	let done_col = if after_len == 0 {
		before_end
	} else {
		after_end
	};

	while byte_pos < line_len && current_col < done_col {
		if bytes[byte_pos] == crate::ESC_U8 {
			if let Some(seq_len) = ansi_seq_len_bytes(bytes, byte_pos) {
				let seq = &bytes[byte_pos..byte_pos + seq_len];
				if is_sgr_bytes(seq) {
					state.apply_sgr_bytes(&seq[2..seq_len - 1]);
				}

				if current_col < before_end {
					pending_before_ansi.push((byte_pos, seq_len));
				} else if current_col >= after_start && current_col < after_end && after_started {
					after.push_str(&line[byte_pos..byte_pos + seq_len]);
				}

				byte_pos += seq_len;
				continue;
			}

			if current_col < before_end {
				before.push('\x1b');
			} else if current_col >= after_start && current_col < after_end && after_started {
				after.push('\x1b');
			}
			byte_pos += 1;
			continue;
		}

		let start = byte_pos;
		while byte_pos < line_len && bytes[byte_pos] != crate::ESC_U8 {
			byte_pos += 1;
		}
		let seg = &line[start..byte_pos];

		use unicode_segmentation::UnicodeSegmentation;
		for g in seg.graphemes(true) {
			if current_col >= done_col {
				break;
			}
			let gw = grapheme_width_str(g);

			if current_col < before_end {
				if !pending_before_ansi.is_empty() {
					for &(p, l) in &pending_before_ansi {
						before.push_str(&line[p..p + l]);
					}
					pending_before_ansi.clear();
				}
				before.push_str(g);
				before_w += gw;
			} else if current_col >= after_start && current_col < after_end {
				let fits = !strict_after || current_col + gw <= after_end;
				if fits {
					if !after_started {
						state.write_restore_str(&mut after);
						after_started = true;
					}
					after.push_str(g);
					after_w += gw;
				}
			}

			current_col += gw;
		}
	}

	ExtractSegmentsResult { before, before_width: before_w, after, after_width: after_w }
}

#[cfg(test)]
mod tests {
	use super::*;

	fn to_u16(s: &str) -> Vec<u16> {
		s.encode_utf16().collect()
	}

	#[test]
	fn test_slice_basic_u16() {
		let data = to_u16("hello world");
		let result = slice_with_width_u16(&data, 0, 5, false);
		assert_eq!(String::from_utf16_lossy(&result.text), "hello");
		assert_eq!(result.width, 5);
	}

	#[test]
	fn test_slice_basic_str() {
		let result = slice_with_width_str("hello world", 0, 5, false);
		assert_eq!(result.text, "hello");
		assert_eq!(result.width, 5);
	}

	#[test]
	fn test_slice_with_ansi_u16() {
		let data = to_u16("\x1b[31mhello\x1b[0m world");
		let result = slice_with_width_u16(&data, 0, 5, false);
		assert_eq!(String::from_utf16_lossy(&result.text), "\x1b[31mhello\x1b[0m");
		assert_eq!(result.width, 5);
	}

	#[test]
	fn test_slice_with_ansi_str() {
		let result = slice_with_width_str("\x1b[31mhello\x1b[0m world", 0, 5, false);
		assert_eq!(result.text, "\x1b[31mhello\x1b[0m");
		assert_eq!(result.width, 5);
	}

	#[test]
	fn test_slice_mid_range() {
		let result = slice_with_width_str("hello world", 6, 5, false);
		assert_eq!(result.text, "world");
		assert_eq!(result.width, 5);
	}

	#[test]
	fn test_extract_segments_basic() {
		let data = to_u16("hello world 12345");
		let r = extract_segments_u16(&data, 5, 11, 6, false);
		assert_eq!(String::from_utf16_lossy(&r.before), "hello");
		assert_eq!(r.before_width, 5);
		assert_eq!(String::from_utf16_lossy(&r.after), " 12345");
		assert_eq!(r.after_width, 6);
	}

	#[test]
	fn test_extract_segments_str() {
		let r = extract_segments_str("hello world 12345", 5, 11, 6, false);
		assert_eq!(r.before, "hello");
		assert_eq!(r.before_width, 5);
		assert_eq!(r.after, " 12345");
		assert_eq!(r.after_width, 6);
	}
}
