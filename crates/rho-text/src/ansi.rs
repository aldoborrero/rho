//! ANSI state tracking and sequence detection.
//!
//! Zero-allocation SGR state machine that tracks foreground/background colors
//! and text attributes across escape sequences.

// ============================================================================
// Attribute flags
// ============================================================================

const ATTR_BOLD: u16 = 1 << 0;
const ATTR_DIM: u16 = 1 << 1;
const ATTR_ITALIC: u16 = 1 << 2;
const ATTR_UNDERLINE: u16 = 1 << 3;
const ATTR_BLINK: u16 = 1 << 4;
const ATTR_INVERSE: u16 = 1 << 6;
const ATTR_HIDDEN: u16 = 1 << 7;
const ATTR_STRIKE: u16 = 1 << 8;

pub(crate) type ColorVal = u32;
pub(crate) const COLOR_NONE: ColorVal = 0;

// ============================================================================
// AnsiState
// ============================================================================

/// Tracks the current SGR (Select Graphic Rendition) state.
///
/// Stores text attributes (bold, italic, …), foreground color, and background
/// color. Can restore from a known state without a full reset.
#[derive(Clone, Copy, Default)]
pub struct AnsiState {
	pub(crate) attrs: u16,
	pub(crate) fg:    ColorVal,
	pub(crate) bg:    ColorVal,
}

impl AnsiState {
	/// Create a new empty state.
	#[inline]
	pub const fn new() -> Self {
		Self { attrs: 0, fg: COLOR_NONE, bg: COLOR_NONE }
	}

	/// Returns true if no attributes or colors are set.
	#[inline]
	pub const fn is_empty(&self) -> bool {
		self.attrs == 0 && self.fg == COLOR_NONE && self.bg == COLOR_NONE
	}

	/// Reset to default (no attributes, no colors).
	#[inline]
	pub const fn reset(&mut self) {
		*self = Self::new();
	}

	/// Returns true if the underline attribute is set.
	#[inline]
	pub const fn has_underline(&self) -> bool {
		self.attrs & ATTR_UNDERLINE != 0
	}

	/// Apply SGR parameters from a UTF-8 byte slice (between `\x1b[` and `m`).
	pub fn apply_sgr_bytes(&mut self, params: &[u8]) {
		if params.is_empty() {
			self.reset();
			return;
		}

		let mut i = 0;
		while i < params.len() {
			let (code, next_i) = parse_sgr_num_bytes(params, i);
			i = next_i;

			match code {
				0 => self.reset(),
				1 => self.attrs |= ATTR_BOLD,
				2 => self.attrs |= ATTR_DIM,
				3 => self.attrs |= ATTR_ITALIC,
				4 => self.attrs |= ATTR_UNDERLINE,
				5 => self.attrs |= ATTR_BLINK,
				7 => self.attrs |= ATTR_INVERSE,
				8 => self.attrs |= ATTR_HIDDEN,
				9 => self.attrs |= ATTR_STRIKE,

				21 => self.attrs &= !ATTR_BOLD,
				22 => self.attrs &= !(ATTR_BOLD | ATTR_DIM),
				23 => self.attrs &= !ATTR_ITALIC,
				24 => self.attrs &= !ATTR_UNDERLINE,
				25 => self.attrs &= !ATTR_BLINK,
				27 => self.attrs &= !ATTR_INVERSE,
				28 => self.attrs &= !ATTR_HIDDEN,
				29 => self.attrs &= !ATTR_STRIKE,

				30..=37 => self.fg = (code - 29) as ColorVal,
				39 => self.fg = COLOR_NONE,
				40..=47 => self.bg = (code - 39) as ColorVal,
				49 => self.bg = COLOR_NONE,
				90..=97 => self.fg = (code - 81) as ColorVal,
				100..=107 => self.bg = (code - 91) as ColorVal,

				38 | 48 => {
					let (mode, ni) = parse_sgr_num_bytes(params, i);
					i = ni;

					let color = match mode {
						5 => {
							let (idx, ni) = parse_sgr_num_bytes(params, i);
							i = ni;
							0x100 | (idx as ColorVal & 0xff)
						},
						2 => {
							let (r, ni) = parse_sgr_num_bytes(params, i);
							let (g, ni) = parse_sgr_num_bytes(params, ni);
							let (b, ni) = parse_sgr_num_bytes(params, ni);
							i = ni;
							0x1000000
								| ((r as ColorVal & 0xff) << 16)
								| ((g as ColorVal & 0xff) << 8)
								| (b as ColorVal & 0xff)
						},
						_ => continue,
					};

					if code == 38 {
						self.fg = color;
					} else {
						self.bg = color;
					}
				},

				_ => {},
			}
		}
	}

	/// Emit UTF-8 escape codes to restore this state from a reset terminal.
	pub fn write_restore_str(&self, out: &mut String) {
		if self.is_empty() {
			return;
		}

		out.push_str("\x1b[");
		let mut first = true;

		macro_rules! push_code {
			($code:expr) => {{
				if !first {
					out.push(';');
				}
				first = false;
				write_u32_str(out, $code);
			}};
		}

		if self.attrs & ATTR_BOLD != 0 {
			push_code!(1);
		}
		if self.attrs & ATTR_DIM != 0 {
			push_code!(2);
		}
		if self.attrs & ATTR_ITALIC != 0 {
			push_code!(3);
		}
		if self.attrs & ATTR_UNDERLINE != 0 {
			push_code!(4);
		}
		if self.attrs & ATTR_BLINK != 0 {
			push_code!(5);
		}
		if self.attrs & ATTR_INVERSE != 0 {
			push_code!(7);
		}
		if self.attrs & ATTR_HIDDEN != 0 {
			push_code!(8);
		}
		if self.attrs & ATTR_STRIKE != 0 {
			push_code!(9);
		}

		write_color_str(out, self.fg, 38, &mut first);
		write_color_str(out, self.bg, 48, &mut first);

		out.push('m');
	}
}

// ============================================================================
// ANSI Sequence Detection
// ============================================================================

/// Returns the length of the ANSI escape sequence starting at `pos` in a UTF-8
/// byte slice, or `None`.
#[inline]
pub fn ansi_seq_len_bytes(data: &[u8], pos: usize) -> Option<usize> {
	if pos >= data.len() || data[pos] != crate::ESC_U8 {
		return None;
	}
	if pos + 1 >= data.len() {
		return None;
	}

	match data[pos + 1] {
		b'[' => {
			// CSI
			for (i, b) in data[pos + 2..].iter().enumerate() {
				if (0x40..=0x7e).contains(b) {
					return Some(i + 3);
				}
			}
			None
		},
		b']' => {
			// OSC
			for (i, &b) in data[pos + 2..].iter().enumerate() {
				if b == 0x07 {
					return Some(i + 3);
				}
				if b == crate::ESC_U8 && data.get(pos + 2 + i + 1) == Some(&b'\\') {
					return Some(i + 4);
				}
			}
			None
		},
		b'P' | b'X' | b'^' | b'_' => {
			// DCS, SOS, PM, APC (terminated by ST or BEL)
			for (i, &b) in data[pos + 2..].iter().enumerate() {
				if b == 0x07 {
					return Some(i + 3);
				}
				if b == crate::ESC_U8 && data.get(pos + 2 + i + 1) == Some(&b'\\') {
					return Some(i + 4);
				}
			}
			None
		},
		0x20..=0x2f => {
			// ESC + intermediates + final byte
			for (i, b) in data[pos + 2..].iter().enumerate() {
				if (0x30..=0x7e).contains(b) {
					return Some(i + 3);
				}
			}
			None
		},
		0x40..=0x7e => Some(2),
		_ => None,
	}
}

/// Check whether a UTF-8 escape sequence is an SGR sequence.
#[inline]
pub fn is_sgr_bytes(seq: &[u8]) -> bool {
	seq.len() >= 3 && seq[1] == b'[' && *seq.last().unwrap() == b'm'
}

// ============================================================================
// SGR number parsing
// ============================================================================

#[inline]
pub(crate) fn parse_sgr_num_bytes(params: &[u8], mut i: usize) -> (u32, usize) {
	while i < params.len() && params[i] == b';' {
		i += 1;
	}

	let mut val: u32 = 0;
	while i < params.len() {
		let b = params[i];
		if b == b';' {
			i += 1;
			break;
		}
		if b.is_ascii_digit() {
			val = val.saturating_mul(10).saturating_add(u32::from(b - b'0'));
		}
		i += 1;
	}
	(val, i)
}

// ============================================================================
// Numeric writing helpers
// ============================================================================

#[inline]
pub(crate) fn write_u32_str(out: &mut String, mut val: u32) {
	if val == 0 {
		out.push('0');
		return;
	}
	let start = out.len();
	while val > 0 {
		out.push(char::from(b'0' + (val % 10) as u8));
		val /= 10;
	}
	// SAFETY: we only pushed ASCII digits
	unsafe {
		out.as_mut_vec()[start..].reverse();
	}
}

// ============================================================================
// Color writing helpers
// ============================================================================

#[inline]
pub(crate) fn write_color_str(out: &mut String, color: ColorVal, base: u32, first: &mut bool) {
	if color == COLOR_NONE {
		return;
	}

	if !*first {
		out.push(';');
	}
	*first = false;

	if color < 0x100 {
		let code = if color <= 8 { color + 29 } else { color + 81 };
		let code = if base == 48 { code + 10 } else { code };
		write_u32_str(out, code);
	} else if color < 0x1000000 {
		write_u32_str(out, base);
		out.push_str(";5;");
		write_u32_str(out, color & 0xff);
	} else {
		write_u32_str(out, base);
		out.push_str(";2;");
		write_u32_str(out, (color >> 16) & 0xff);
		out.push(';');
		write_u32_str(out, (color >> 8) & 0xff);
		out.push(';');
		write_u32_str(out, color & 0xff);
	}
}

/// Update ANSI state by scanning all SGR sequences in a UTF-8 text span.
pub fn update_state_from_text_bytes(data: &[u8], state: &mut AnsiState) {
	let mut i = 0usize;
	while i < data.len() {
		if data[i] == crate::ESC_U8
			&& let Some(seq_len) = ansi_seq_len_bytes(data, i)
		{
			let seq = &data[i..i + seq_len];
			if is_sgr_bytes(seq) {
				state.apply_sgr_bytes(&seq[2..seq_len - 1]);
			}
			i += seq_len;
			continue;
		}
		i += 1;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_ansi_detection_bytes() {
		let data = b"\x1b[31mred\x1b[0m";
		assert_eq!(ansi_seq_len_bytes(data, 0), Some(5));
		assert_eq!(ansi_seq_len_bytes(data, 8), Some(4));
	}

	#[test]
	fn test_ansi_state_sgr() {
		let mut state = AnsiState::new();
		assert!(state.is_empty());

		// Bold
		state.apply_sgr_bytes(b"1");
		assert!(!state.is_empty());
		assert_eq!(state.attrs & 1, 1);

		// Reset
		state.apply_sgr_bytes(b"0");
		assert!(state.is_empty());
	}

	#[test]
	fn test_write_restore_str() {
		let mut state = AnsiState::new();
		state.apply_sgr_bytes(b"38;2;156;163;176");
		let mut out = String::new();
		state.write_restore_str(&mut out);
		assert_eq!(out, "\x1b[38;2;156;163;176m");
	}

	#[test]
	fn test_is_sgr() {
		assert!(is_sgr_bytes(b"\x1b[31m"));
		assert!(is_sgr_bytes(b"\x1b[0m"));
		assert!(!is_sgr_bytes(b"\x1b[A"));
	}
}
