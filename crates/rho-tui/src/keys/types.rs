//! Key types and constants.

// Internal sentinel codes for CSI 1;mod <letter> forms:
pub const ARROW_UP: i32 = -1;
pub const ARROW_DOWN: i32 = -2;
pub const ARROW_RIGHT: i32 = -3;
pub const ARROW_LEFT: i32 = -4;

pub const FUNC_DELETE: i32 = -10;
pub const FUNC_INSERT: i32 = -11;
pub const FUNC_PAGE_UP: i32 = -12;
pub const FUNC_PAGE_DOWN: i32 = -13;
pub const FUNC_HOME: i32 = -14;
pub const FUNC_END: i32 = -15;
pub const FUNC_CLEAR: i32 = -16;

pub const FUNC_F1: i32 = -20;
pub const FUNC_F2: i32 = -21;
pub const FUNC_F3: i32 = -22;
pub const FUNC_F4: i32 = -23;
pub const FUNC_F5: i32 = -24;
pub const FUNC_F6: i32 = -25;
pub const FUNC_F7: i32 = -26;
pub const FUNC_F8: i32 = -27;
pub const FUNC_F9: i32 = -28;
pub const FUNC_F10: i32 = -29;
pub const FUNC_F11: i32 = -30;
pub const FUNC_F12: i32 = -31;

pub const CP_ESCAPE: i32 = 27;
pub const CP_TAB: i32 = 9;
pub const CP_ENTER: i32 = 13;
pub const CP_SPACE: i32 = 32;
pub const CP_BACKSPACE: i32 = 127;
pub const CP_KP_ENTER: i32 = 57414;
pub const CP_KP_0: i32 = 57399;
pub const CP_KP_1: i32 = 57400;
pub const CP_KP_2: i32 = 57401;
pub const CP_KP_3: i32 = 57402;
pub const CP_KP_4: i32 = 57403;
pub const CP_KP_5: i32 = 57404;
pub const CP_KP_6: i32 = 57405;
pub const CP_KP_7: i32 = 57406;
pub const CP_KP_8: i32 = 57407;
pub const CP_KP_9: i32 = 57408;
pub const CP_KP_DECIMAL: i32 = 57409;

pub const MOD_SHIFT: u32 = 1;
pub const MOD_ALT: u32 = 2;
pub const MOD_CTRL: u32 = 4;

pub const LOCK_MASK: u32 = 64 + 128;

/// Parsed Kitty keyboard protocol sequence (internal).
pub struct ParsedKittySequence {
	pub codepoint:       i32,
	pub shifted_key:     Option<i32>,
	pub base_layout_key: Option<i32>,
	pub text_codepoint:  Option<i32>,
	pub modifier:        u32,
	pub event_type:      Option<u32>,
}

/// Parsed Kitty keyboard protocol result (public API).
pub struct ParsedKittyResult {
	pub codepoint:       i32,
	pub shifted_key:     Option<i32>,
	pub base_layout_key: Option<i32>,
	pub modifier:        u32,
	pub event_type:      Option<u32>,
}

/// Map keypad navigation keys to their functional equivalents.
#[inline]
pub const fn map_keypad_nav(codepoint: i32) -> Option<i32> {
	match codepoint {
		CP_KP_0 => Some(FUNC_INSERT),
		CP_KP_1 => Some(FUNC_END),
		CP_KP_2 => Some(ARROW_DOWN),
		CP_KP_3 => Some(FUNC_PAGE_DOWN),
		CP_KP_4 => Some(ARROW_LEFT),
		CP_KP_5 => Some(FUNC_CLEAR),
		CP_KP_6 => Some(ARROW_RIGHT),
		CP_KP_7 => Some(FUNC_HOME),
		CP_KP_8 => Some(ARROW_UP),
		CP_KP_9 => Some(FUNC_PAGE_UP),
		CP_KP_DECIMAL => Some(FUNC_DELETE),
		_ => None,
	}
}

/// Check if a codepoint corresponds to a known symbol key.
#[inline]
pub const fn is_symbol_key(cp: i32) -> bool {
	matches!(
		cp,
		96 | 34
			| 45 | 61
			| 91 | 93
			| 92 | 59
			| 39 | 44
			| 46 | 47
			| 33 | 64
			| 35 | 36
			| 37 | 94
			| 38 | 42
			| 40 | 41
			| 95 | 43
			| 124 | 126
			| 123 | 125
			| 58 | 60
			| 62 | 63
	)
}

/// Pre-allocated single ASCII printable characters (33-126).
pub static ASCII_PRINTABLE: [&str; 94] = [
	"!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/", "0", "1", "2", "3",
	"4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?", "@", "A", "B", "C", "D", "E", "F",
	"G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y",
	"Z", "[", "\\", "]", "^", "_", "`", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l",
	"m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "{", "|", "}", "~",
];

pub static CTRL_LETTERS: [&str; 26] = [
	"ctrl+a", "ctrl+b", "ctrl+c", "ctrl+d", "ctrl+e", "ctrl+f", "ctrl+g", "ctrl+h", "ctrl+i",
	"ctrl+j", "ctrl+k", "ctrl+l", "ctrl+m", "ctrl+n", "ctrl+o", "ctrl+p", "ctrl+q", "ctrl+r",
	"ctrl+s", "ctrl+t", "ctrl+u", "ctrl+v", "ctrl+w", "ctrl+x", "ctrl+y", "ctrl+z",
];

pub static ALT_LETTERS: [&str; 26] = [
	"alt+a", "alt+b", "alt+c", "alt+d", "alt+e", "alt+f", "alt+g", "alt+h", "alt+i", "alt+j",
	"alt+k", "alt+l", "alt+m", "alt+n", "alt+o", "alt+p", "alt+q", "alt+r", "alt+s", "alt+t",
	"alt+u", "alt+v", "alt+w", "alt+x", "alt+y", "alt+z",
];

pub static CTRL_ALT_LETTERS: [&str; 26] = [
	"ctrl+alt+a",
	"ctrl+alt+b",
	"ctrl+alt+c",
	"ctrl+alt+d",
	"ctrl+alt+e",
	"ctrl+alt+f",
	"ctrl+alt+g",
	"ctrl+alt+h",
	"ctrl+alt+i",
	"ctrl+alt+j",
	"ctrl+alt+k",
	"ctrl+alt+l",
	"ctrl+alt+m",
	"ctrl+alt+n",
	"ctrl+alt+o",
	"ctrl+alt+p",
	"ctrl+alt+q",
	"ctrl+alt+r",
	"ctrl+alt+s",
	"ctrl+alt+t",
	"ctrl+alt+u",
	"ctrl+alt+v",
	"ctrl+alt+w",
	"ctrl+alt+x",
	"ctrl+alt+y",
	"ctrl+alt+z",
];

pub static LETTERS: [&str; 26] = [
	"a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
	"t", "u", "v", "w", "x", "y", "z",
];

// Digit parsing helpers

#[inline]
pub fn parse_digits(bytes: &[u8], mut idx: usize, end: usize) -> Option<(u32, usize)> {
	if idx >= end || !bytes[idx].is_ascii_digit() {
		return None;
	}

	let mut value: u32 = 0;
	while idx < end && bytes[idx].is_ascii_digit() {
		value = value
			.checked_mul(10)?
			.checked_add(u32::from(bytes[idx] - b'0'))?;
		idx += 1;
	}

	Some((value, idx))
}

#[inline]
pub fn parse_optional_digits(bytes: &[u8], idx: usize, end: usize) -> (Option<u32>, usize) {
	if idx >= end || !bytes[idx].is_ascii_digit() {
		return (None, idx);
	}
	parse_digits(bytes, idx, end).map_or((None, idx), |(v, i)| (Some(v), i))
}
