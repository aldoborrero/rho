//! Core Kitty protocol parsing and key identification.

use std::borrow::Cow;

use super::{legacy::LEGACY_SEQUENCES, types::*};

/// Parse terminal input and return a normalized key identifier.
///
/// Returns a key id like "escape" or "ctrl+c", or None if unrecognized.
pub fn parse_key(bytes: &[u8], kitty_protocol_active: bool) -> Option<Cow<'static, str>> {
	// Fast path: single byte (most common for typing)
	if bytes.len() == 1 {
		return parse_single_byte(bytes[0]);
	}

	// All escape sequences start with ESC
	if bytes.first() != Some(&0x1b) {
		return None;
	}

	// O(1) lookup in perfect hash map for legacy sequences
	if let Some(&key_id) = LEGACY_SEQUENCES.get(bytes) {
		return Some(Cow::Borrowed(key_id));
	}

	// xterm modifyOtherKeys (CSI 27;...;...~)
	if let Some((mods, keycode)) = parse_modify_other_keys(bytes) {
		let key_name = format_key_name(keycode)?;
		if mods == 0 {
			return Some(Cow::Borrowed(key_name));
		}
		return Some(Cow::Owned(format_with_mods(mods & !LOCK_MASK, key_name)));
	}

	// Try Kitty protocol sequences
	if let Some(parsed) = parse_kitty_sequence(bytes) {
		return format_kitty_key(&parsed);
	}

	// Two-byte ESC sequences
	if bytes.len() == 2 {
		return parse_esc_pair(bytes[1], kitty_protocol_active);
	}

	// Fixed CSI / SS3 sequences not covered by LEGACY_SEQUENCES
	match bytes {
		b"\x1b[Z" => Some(Cow::Borrowed("shift+tab")),
		b"\x1bOM" => Some(Cow::Borrowed("enter")),
		_ => None,
	}
}

#[inline]
fn parse_single_byte(code: u8) -> Option<Cow<'static, str>> {
	match code {
		0x1b => Some(Cow::Borrowed("escape")),
		b'\t' => Some(Cow::Borrowed("tab")),
		b'\r' | b'\n' => Some(Cow::Borrowed("enter")),
		0x00 => Some(Cow::Borrowed("ctrl+space")),
		b' ' => Some(Cow::Borrowed("space")),
		0x7f | 0x08 => Some(Cow::Borrowed("backspace")),
		28 => Some(Cow::Borrowed("ctrl+\\")),
		29 => Some(Cow::Borrowed("ctrl+]")),
		30 => Some(Cow::Borrowed("ctrl+^")),
		31 => Some(Cow::Borrowed("ctrl+_")),
		1..=26 => Some(Cow::Borrowed(CTRL_LETTERS[(code - 1) as usize])),
		b'a'..=b'z' => Some(Cow::Borrowed(LETTERS[(code - b'a') as usize])),
		33..=126 => Some(Cow::Borrowed(ASCII_PRINTABLE[(code - 33) as usize])),
		_ => None,
	}
}

#[inline]
fn parse_esc_pair(code: u8, kitty_protocol_active: bool) -> Option<Cow<'static, str>> {
	match code {
		0x7f | 0x08 => return Some(Cow::Borrowed("alt+backspace")),
		b'\r' => return Some(Cow::Borrowed("alt+enter")),
		b'\t' => return Some(Cow::Borrowed("alt+tab")),
		_ => {},
	}

	if !kitty_protocol_active {
		match code {
			b' ' => return Some(Cow::Borrowed("alt+space")),
			b'B' => return Some(Cow::Borrowed("alt+left")),
			b'F' => return Some(Cow::Borrowed("alt+right")),
			1..=26 => return Some(Cow::Borrowed(CTRL_ALT_LETTERS[(code - 1) as usize])),
			b'a'..=b'z' => return Some(Cow::Borrowed(ALT_LETTERS[(code - b'a') as usize])),
			_ => {},
		}
	}

	None
}

// =============================================================================
// Kitty Protocol Parsing
// =============================================================================

/// Parse a Kitty keyboard protocol sequence from raw bytes.
pub fn parse_kitty_sequence(bytes: &[u8]) -> Option<ParsedKittySequence> {
	if bytes.len() < 4 || bytes[0] != 0x1b || bytes[1] != b'[' {
		return None;
	}

	match *bytes.last()? {
		b'u' => parse_csi_u(bytes),
		b'~' => parse_functional(bytes),
		b'A' | b'B' | b'C' | b'D' | b'E' | b'F' | b'H' | b'P' | b'Q' | b'R' | b'S' => {
			parse_csi_1_letter(bytes)
		},
		_ => None,
	}
}

fn parse_csi_u(bytes: &[u8]) -> Option<ParsedKittySequence> {
	let end = bytes.len() - 1;
	let mut idx = 2;

	let (codepoint_u32, next_idx) = parse_digits(bytes, idx, end)?;
	let codepoint = i32::try_from(codepoint_u32).ok()?;
	idx = next_idx;

	let mut shifted_key = None;
	let mut base_layout_key = None;
	if idx < end && bytes[idx] == b':' {
		idx += 1;

		let (shifted_value, next_idx) = parse_optional_digits(bytes, idx, end);
		shifted_key = shifted_value.and_then(|v| i32::try_from(v).ok());
		idx = next_idx;

		if idx < end && bytes[idx] == b':' {
			idx += 1;
			let (base_value, next_idx) = parse_digits(bytes, idx, end)?;
			base_layout_key = Some(i32::try_from(base_value).ok()?);
			idx = next_idx;
		}
	}

	let mut mod_value: u32 = 1;
	let mut event_type: Option<u32> = None;

	if idx < end && bytes[idx] == b';' {
		idx += 1;

		if idx < end && bytes[idx].is_ascii_digit() {
			let (v, next_idx) = parse_digits(bytes, idx, end)?;
			mod_value = v;
			idx = next_idx;
		} else {
			mod_value = 1;
		}

		if idx < end && bytes[idx] == b':' {
			idx += 1;
			let (ev, next_idx) = parse_digits(bytes, idx, end)?;
			event_type = Some(ev);
			idx = next_idx;
		}
	}

	let mut text_codepoint: Option<i32> = None;
	let mut text_count: u32 = 0;
	if idx < end && bytes[idx] == b';' {
		idx += 1;
		while idx < end {
			if bytes[idx] == b':' {
				idx += 1;
				continue;
			}
			let (cp, next_idx) = parse_digits(bytes, idx, end)?;
			text_count += 1;
			if text_count == 1 {
				if cp >= 32 {
					let cp_i32 = i32::try_from(cp).ok();
					if let Some(value) = cp_i32
						&& char::from_u32(cp).is_some()
					{
						text_codepoint = Some(value);
					}
				}
			} else {
				text_codepoint = None;
			}
			idx = next_idx;
			if idx < end && bytes[idx] == b':' {
				idx += 1;
			}
		}
	}

	if idx != end || mod_value == 0 {
		return None;
	}

	Some(ParsedKittySequence {
		codepoint,
		shifted_key,
		base_layout_key,
		text_codepoint,
		modifier: mod_value - 1,
		event_type,
	})
}

fn parse_csi_1_letter(bytes: &[u8]) -> Option<ParsedKittySequence> {
	if !bytes.starts_with(b"\x1b[1;") {
		return None;
	}

	let end = bytes.len();
	let mut idx = 4;
	let (mod_value, next_idx) = parse_digits(bytes, idx, end)?;
	idx = next_idx;

	let mut event_type = None;
	if idx < end && bytes[idx] == b':' {
		idx += 1;
		let (ev, next_idx) = parse_digits(bytes, idx, end)?;
		event_type = Some(ev);
		idx = next_idx;
	}

	if idx + 1 != end || mod_value == 0 {
		return None;
	}

	let codepoint = match bytes[idx] {
		b'A' => ARROW_UP,
		b'B' => ARROW_DOWN,
		b'C' => ARROW_RIGHT,
		b'D' => ARROW_LEFT,
		b'H' => FUNC_HOME,
		b'F' => FUNC_END,
		b'E' => FUNC_CLEAR,
		b'P' => FUNC_F1,
		b'Q' => FUNC_F2,
		b'R' => FUNC_F3,
		b'S' => FUNC_F4,
		_ => return None,
	};

	Some(ParsedKittySequence {
		codepoint,
		shifted_key: None,
		base_layout_key: None,
		text_codepoint: None,
		modifier: mod_value - 1,
		event_type,
	})
}

fn parse_functional(bytes: &[u8]) -> Option<ParsedKittySequence> {
	let end = bytes.len() - 1;
	let mut idx = 2;
	let (key_num, next_idx) = parse_digits(bytes, idx, end)?;
	idx = next_idx;

	let mod_value = if idx < end && bytes[idx] == b';' {
		idx += 1;
		let (v, next_idx) = parse_digits(bytes, idx, end)?;
		idx = next_idx;
		v
	} else {
		1
	};

	let mut event_type = None;
	if idx < end && bytes[idx] == b':' {
		idx += 1;
		let (ev, next_idx) = parse_digits(bytes, idx, end)?;
		event_type = Some(ev);
		idx = next_idx;
	}

	if idx != end || mod_value == 0 {
		return None;
	}

	let codepoint = match key_num {
		2 => FUNC_INSERT,
		3 => FUNC_DELETE,
		5 => FUNC_PAGE_UP,
		6 => FUNC_PAGE_DOWN,
		1 | 7 => FUNC_HOME,
		4 | 8 => FUNC_END,
		11 => FUNC_F1,
		12 => FUNC_F2,
		13 => FUNC_F3,
		14 => FUNC_F4,
		15 => FUNC_F5,
		17 => FUNC_F6,
		18 => FUNC_F7,
		19 => FUNC_F8,
		20 => FUNC_F9,
		21 => FUNC_F10,
		23 => FUNC_F11,
		24 => FUNC_F12,
		_ => return None,
	};

	Some(ParsedKittySequence {
		codepoint,
		shifted_key: None,
		base_layout_key: None,
		text_codepoint: None,
		modifier: mod_value - 1,
		event_type,
	})
}

// =============================================================================
// modifyOtherKeys
// =============================================================================

/// Parse xterm "modifyOtherKeys" format:
///   CSI 27 ; modifiers ; keycode ~
pub fn parse_modify_other_keys(bytes: &[u8]) -> Option<(u32, i32)> {
	if bytes.len() < 7 || !bytes.starts_with(b"\x1b[27;") {
		return None;
	}

	let mut end = bytes.len();
	if bytes.last() == Some(&b'~') {
		end -= 1;
	}
	if end <= 5 {
		return None;
	}

	let mut idx = 5;
	let (mod_value, next_idx) = parse_digits(bytes, idx, end)?;
	idx = next_idx;

	if idx >= end || bytes[idx] != b';' {
		return None;
	}
	idx += 1;

	let (keycode_u32, next_idx) = parse_digits(bytes, idx, end)?;
	idx = next_idx;

	if idx != end || mod_value == 0 {
		return None;
	}

	let modifier = mod_value - 1;
	let keycode = i32::try_from(keycode_u32).ok()?;
	Some((modifier, keycode))
}

// =============================================================================
// Formatting
// =============================================================================

pub(crate) fn format_kitty_key(parsed: &ParsedKittySequence) -> Option<Cow<'static, str>> {
	let effective_mod = parsed.modifier & !LOCK_MASK;
	let effective_codepoint = {
		let cp = parsed.codepoint;
		let is_ascii_letter = u8::try_from(cp)
			.ok()
			.is_some_and(|b| b.is_ascii_alphabetic());
		let is_known_symbol = is_symbol_key(cp);
		if is_ascii_letter || is_known_symbol {
			cp
		} else {
			parsed.base_layout_key.unwrap_or(cp)
		}
	};

	if effective_mod == 0 {
		if let Some(text_codepoint) = parsed.text_codepoint
			&& let Some(key_name) = format_key_name(text_codepoint)
		{
			return Some(Cow::Borrowed(key_name));
		}
		return format_key_name(effective_codepoint).map(Cow::Borrowed);
	}

	let key_name = format_key_name(effective_codepoint)?;
	Some(Cow::Owned(format_with_mods(effective_mod, key_name)))
}

/// Map a codepoint to a static key name string.
pub fn format_key_name(codepoint: i32) -> Option<&'static str> {
	match codepoint {
		CP_ESCAPE => Some("escape"),
		CP_TAB => Some("tab"),
		CP_ENTER | CP_KP_ENTER => Some("enter"),
		CP_SPACE => Some("space"),
		CP_BACKSPACE => Some("backspace"),
		CP_KP_0 => Some("insert"),
		CP_KP_1 => Some("end"),
		CP_KP_2 => Some("down"),
		CP_KP_3 => Some("pageDown"),
		CP_KP_4 => Some("left"),
		CP_KP_5 => Some("clear"),
		CP_KP_6 => Some("right"),
		CP_KP_7 => Some("home"),
		CP_KP_8 => Some("up"),
		CP_KP_9 => Some("pageUp"),
		CP_KP_DECIMAL => Some("delete"),

		FUNC_DELETE => Some("delete"),
		FUNC_INSERT => Some("insert"),
		FUNC_HOME => Some("home"),
		FUNC_END => Some("end"),
		FUNC_PAGE_UP => Some("pageUp"),
		FUNC_PAGE_DOWN => Some("pageDown"),
		FUNC_CLEAR => Some("clear"),

		ARROW_UP => Some("up"),
		ARROW_DOWN => Some("down"),
		ARROW_LEFT => Some("left"),
		ARROW_RIGHT => Some("right"),

		FUNC_F1 => Some("f1"),
		FUNC_F2 => Some("f2"),
		FUNC_F3 => Some("f3"),
		FUNC_F4 => Some("f4"),
		FUNC_F5 => Some("f5"),
		FUNC_F6 => Some("f6"),
		FUNC_F7 => Some("f7"),
		FUNC_F8 => Some("f8"),
		FUNC_F9 => Some("f9"),
		FUNC_F10 => Some("f10"),
		FUNC_F11 => Some("f11"),
		FUNC_F12 => Some("f12"),

		33..=126 => Some(ASCII_PRINTABLE[(codepoint - 33) as usize]),
		_ => None,
	}
}

/// Format a key name with modifier prefix.
pub fn format_with_mods(mods: u32, key_name: &str) -> String {
	let mut result = String::with_capacity(16);
	if mods & MOD_SHIFT != 0 {
		result.push_str("shift+");
	}
	if mods & MOD_CTRL != 0 {
		result.push_str("ctrl+");
	}
	if mods & MOD_ALT != 0 {
		result.push_str("alt+");
	}
	result.push_str(key_name);
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_single_byte() {
		assert_eq!(parse_key(b"\x1b", false).as_deref(), Some("escape"));
		assert_eq!(parse_key(b"\t", false).as_deref(), Some("tab"));
		assert_eq!(parse_key(b"\r", false).as_deref(), Some("enter"));
		assert_eq!(parse_key(b" ", false).as_deref(), Some("space"));
		assert_eq!(parse_key(b"\x7f", false).as_deref(), Some("backspace"));
		assert_eq!(parse_key(b"\x03", false).as_deref(), Some("ctrl+c"));
		assert_eq!(parse_key(b"a", false).as_deref(), Some("a"));
	}

	#[test]
	fn test_parse_legacy_sequences() {
		assert_eq!(parse_key(b"\x1b[A", false).as_deref(), Some("up"));
		assert_eq!(parse_key(b"\x1b[B", false).as_deref(), Some("down"));
		assert_eq!(parse_key(b"\x1bOP", false).as_deref(), Some("f1"));
		assert_eq!(parse_key(b"\x1b[3~", false).as_deref(), Some("delete"));
	}

	#[test]
	fn test_parse_kitty_csi_u() {
		// CSI 65;5u => ctrl+a (codepoint 65='A', modifier 5=ctrl)
		let result = parse_key(b"\x1b[65;5u", true);
		assert_eq!(result.as_deref(), Some("ctrl+A"));
	}

	#[test]
	fn test_parse_esc_pair_legacy() {
		assert_eq!(parse_key(b"\x1b\x7f", false).as_deref(), Some("alt+backspace"));
		assert_eq!(parse_key(b"\x1b\r", false).as_deref(), Some("alt+enter"));
		assert_eq!(parse_key(b"\x1ba", false).as_deref(), Some("alt+a"));
	}

	#[test]
	fn test_parse_kitty_sequence() {
		let parsed = parse_kitty_sequence(b"\x1b[97;1u").unwrap();
		assert_eq!(parsed.codepoint, 97);
		assert_eq!(parsed.modifier, 0);
	}

	#[test]
	fn test_parse_modify_other_keys() {
		let (mods, keycode) = parse_modify_other_keys(b"\x1b[27;5;97~").unwrap();
		assert_eq!(mods, 4); // ctrl
		assert_eq!(keycode, 97); // 'a'
	}
}
