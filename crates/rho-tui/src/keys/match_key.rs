//! Key matching: compare raw terminal input against a key identifier string.

use super::{
	legacy::{LEGACY_SEQUENCES, matches_legacy_key, matches_legacy_modifier_sequence},
	parse::{parse_kitty_sequence, parse_modify_other_keys},
	types::*,
};

struct ParsedKeyId<'a> {
	key:      &'a str,
	modifier: u32,
}

fn parse_key_id(key_id: &str) -> Option<ParsedKeyId<'_>> {
	let s = key_id.trim();
	if s.is_empty() {
		return None;
	}

	let (prefix, forced_key_plus): (&str, bool) = if s == "+" {
		("", true)
	} else if let Some(stripped) = s.strip_suffix("++") {
		(stripped, true)
	} else {
		(s, false)
	};

	let mut modifier = 0;
	let mut key: Option<&str> = if forced_key_plus { Some("+") } else { None };

	for part in prefix.split('+') {
		let p = part.trim();
		let [c0, ..] = p.as_bytes() else {
			continue;
		};

		match c0 {
			b'c' | b'C' => {
				if p.eq_ignore_ascii_case("ctrl") {
					modifier |= MOD_CTRL;
					continue;
				}
			},
			b's' | b'S' => {
				if p.eq_ignore_ascii_case("shift") {
					modifier |= MOD_SHIFT;
					continue;
				}
			},
			b'a' | b'A' => {
				if p.eq_ignore_ascii_case("alt") {
					modifier |= MOD_ALT;
					continue;
				}
			},
			_ => {},
		}

		key = Some(p);
	}

	let mut key = key?;
	if key.eq_ignore_ascii_case("plus") {
		key = "+";
	} else if key.eq_ignore_ascii_case("esc") {
		key = "esc";
	}

	Some(ParsedKeyId { key, modifier })
}

#[inline]
const fn raw_ctrl_char(letter: u8) -> u8 {
	(letter.to_ascii_lowercase() - b'a') + 1
}

const fn ctrl_symbol_to_byte(symbol: u8) -> Option<u8> {
	match symbol {
		b'@' | b'[' | b'\\' | b']' | b'^' | b'_' => Some(symbol - 0x40),
		b'-' => Some(0x1f),
		_ => None,
	}
}

/// Match input data against a key identifier string.
///
/// Returns true when the bytes represent the specified key with modifiers.
pub fn matches_key(bytes: &[u8], key_id: &str, kitty_protocol_active: bool) -> bool {
	let Some(ParsedKeyId { key, modifier }) = parse_key_id(key_id) else {
		return false;
	};

	let kitty_parsed = parse_kitty_sequence(bytes);
	let kitty_matches = |codepoint: i32, m: u32| -> bool {
		let Some(p) = kitty_parsed.as_ref() else {
			return false;
		};
		let actual_mod = p.modifier & !LOCK_MASK;
		let expected_mod = m & !LOCK_MASK;
		if actual_mod != expected_mod {
			return false;
		}
		let mut parsed_codepoint = p.codepoint;
		let mut parsed_base = p.base_layout_key;
		if p.text_codepoint.is_none() {
			if let Some(mapped) = map_keypad_nav(parsed_codepoint) {
				parsed_codepoint = mapped;
			}
			if let Some(base) = parsed_base
				&& let Some(mapped) = map_keypad_nav(base)
			{
				parsed_base = Some(mapped);
			}
		}
		if parsed_codepoint == codepoint {
			return true;
		}
		if let Some(base) = parsed_base
			&& base == codepoint
		{
			let is_ascii_letter = u8::try_from(parsed_codepoint)
				.ok()
				.is_some_and(|b| b.is_ascii_alphabetic());
			let is_known_symbol = is_symbol_key(parsed_codepoint);
			if !is_ascii_letter && !is_known_symbol {
				return true;
			}
		}
		false
	};

	let mok = parse_modify_other_keys(bytes);
	let mok_matches =
		|keycode: i32, m: u32| -> bool { mok.is_some_and(|(mm, kk)| kk == keycode && mm == m) };

	// Named keys (case-insensitive)
	if key.eq_ignore_ascii_case("escape") || key.eq_ignore_ascii_case("esc") {
		if modifier != 0 {
			return false;
		}
		return bytes == b"\x1b" || kitty_matches(CP_ESCAPE, 0);
	}

	if key.eq_ignore_ascii_case("space") {
		if modifier == MOD_CTRL && bytes == b"\x00" {
			return true;
		}
		if modifier == MOD_ALT && !kitty_protocol_active && bytes == b"\x1b " {
			return true;
		}
		if modifier == 0 {
			return bytes == b" " || kitty_matches(CP_SPACE, 0);
		}
		return kitty_matches(CP_SPACE, modifier) || mok_matches(CP_SPACE, modifier);
	}

	if key.eq_ignore_ascii_case("tab") {
		if modifier == MOD_SHIFT {
			return bytes == b"\x1b[Z"
				|| kitty_matches(CP_TAB, MOD_SHIFT)
				|| mok_matches(CP_TAB, MOD_SHIFT);
		}
		if modifier == MOD_ALT && bytes == b"\x1b\t" {
			return true;
		}
		if modifier == 0 {
			return bytes == b"\t" || kitty_matches(CP_TAB, 0);
		}
		return kitty_matches(CP_TAB, modifier) || mok_matches(CP_TAB, modifier);
	}

	if key.eq_ignore_ascii_case("enter") || key.eq_ignore_ascii_case("return") {
		if modifier == MOD_ALT && bytes == b"\x1b\r" {
			return true;
		}
		if modifier == 0 {
			return bytes == b"\r"
				|| bytes == b"\n"
				|| bytes == b"\x1bOM"
				|| kitty_matches(CP_ENTER, 0)
				|| kitty_matches(CP_KP_ENTER, 0);
		}
		return kitty_matches(CP_ENTER, modifier)
			|| kitty_matches(CP_KP_ENTER, modifier)
			|| mok_matches(CP_ENTER, modifier)
			|| mok_matches(CP_KP_ENTER, modifier);
	}

	if key.eq_ignore_ascii_case("backspace") {
		if modifier == MOD_ALT {
			return bytes == b"\x1b\x7f"
				|| bytes == b"\x1b\x08"
				|| kitty_matches(CP_BACKSPACE, MOD_ALT)
				|| mok_matches(CP_BACKSPACE, MOD_ALT);
		}
		if modifier == 0 {
			return bytes == b"\x7f" || bytes == b"\x08" || kitty_matches(CP_BACKSPACE, 0);
		}
		return kitty_matches(CP_BACKSPACE, modifier) || mok_matches(CP_BACKSPACE, modifier);
	}

	if key.eq_ignore_ascii_case("insert") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "insert") || kitty_matches(FUNC_INSERT, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "insert", modifier)
			|| kitty_matches(FUNC_INSERT, modifier);
	}

	if key.eq_ignore_ascii_case("delete") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "delete") || kitty_matches(FUNC_DELETE, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "delete", modifier)
			|| kitty_matches(FUNC_DELETE, modifier);
	}

	if key.eq_ignore_ascii_case("clear") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "clear") || kitty_matches(FUNC_CLEAR, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "clear", modifier)
			|| kitty_matches(FUNC_CLEAR, modifier);
	}

	if key.eq_ignore_ascii_case("home") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "home") || kitty_matches(FUNC_HOME, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "home", modifier)
			|| kitty_matches(FUNC_HOME, modifier);
	}

	if key.eq_ignore_ascii_case("end") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "end") || kitty_matches(FUNC_END, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "end", modifier)
			|| kitty_matches(FUNC_END, modifier);
	}

	if key.eq_ignore_ascii_case("pageup") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "pageUp") || kitty_matches(FUNC_PAGE_UP, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "pageUp", modifier)
			|| kitty_matches(FUNC_PAGE_UP, modifier);
	}

	if key.eq_ignore_ascii_case("pagedown") {
		if modifier == 0 {
			return matches_legacy_key(bytes, "pageDown") || kitty_matches(FUNC_PAGE_DOWN, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "pageDown", modifier)
			|| kitty_matches(FUNC_PAGE_DOWN, modifier);
	}

	if key.eq_ignore_ascii_case("up") {
		if modifier == MOD_ALT {
			return bytes == b"\x1bp" || kitty_matches(ARROW_UP, MOD_ALT);
		}
		if modifier == 0 {
			return matches_legacy_key(bytes, "up") || kitty_matches(ARROW_UP, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "up", modifier)
			|| kitty_matches(ARROW_UP, modifier);
	}

	if key.eq_ignore_ascii_case("down") {
		if modifier == MOD_ALT {
			return bytes == b"\x1bn" || kitty_matches(ARROW_DOWN, MOD_ALT);
		}
		if modifier == 0 {
			return matches_legacy_key(bytes, "down") || kitty_matches(ARROW_DOWN, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "down", modifier)
			|| kitty_matches(ARROW_DOWN, modifier);
	}

	if key.eq_ignore_ascii_case("left") {
		if modifier == MOD_ALT {
			return bytes == b"\x1b[1;3D"
				|| (!kitty_protocol_active && bytes == b"\x1bB")
				|| bytes == b"\x1bb"
				|| kitty_matches(ARROW_LEFT, MOD_ALT);
		}
		if modifier == MOD_CTRL {
			return bytes == b"\x1b[1;5D"
				|| matches_legacy_modifier_sequence(bytes, "left", MOD_CTRL)
				|| kitty_matches(ARROW_LEFT, MOD_CTRL);
		}
		if modifier == 0 {
			return matches_legacy_key(bytes, "left") || kitty_matches(ARROW_LEFT, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "left", modifier)
			|| kitty_matches(ARROW_LEFT, modifier);
	}

	if key.eq_ignore_ascii_case("right") {
		if modifier == MOD_ALT {
			return bytes == b"\x1b[1;3C"
				|| (!kitty_protocol_active && bytes == b"\x1bF")
				|| bytes == b"\x1bf"
				|| kitty_matches(ARROW_RIGHT, MOD_ALT);
		}
		if modifier == MOD_CTRL {
			return bytes == b"\x1b[1;5C"
				|| matches_legacy_modifier_sequence(bytes, "right", MOD_CTRL)
				|| kitty_matches(ARROW_RIGHT, MOD_CTRL);
		}
		if modifier == 0 {
			return matches_legacy_key(bytes, "right") || kitty_matches(ARROW_RIGHT, 0);
		}
		return matches_legacy_modifier_sequence(bytes, "right", modifier)
			|| kitty_matches(ARROW_RIGHT, modifier);
	}

	// Function keys
	let f_code = match key.as_bytes() {
		[b'f' | b'F', n @ b'1'..=b'9'] => Some(FUNC_F1 + (n - b'1') as i32),
		[b'f' | b'F', b'1', b'0'] => Some(FUNC_F10),
		[b'f' | b'F', b'1', b'1'] => Some(FUNC_F11),
		[b'f' | b'F', b'1', b'2'] => Some(FUNC_F12),
		_ => None,
	};

	if let Some(cp) = f_code {
		if modifier == 0 {
			return matches_legacy_key(bytes, key);
		}
		return kitty_matches(cp, modifier);
	}

	// Single-character keys
	if let [ch] = key.as_bytes() {
		if !ch.is_ascii_graphic() {
			return false;
		}

		let ch = ch.to_ascii_lowercase();
		let codepoint = ch as i32;
		let is_letter = ch.is_ascii_lowercase();

		if modifier == (MOD_CTRL | MOD_ALT) && !kitty_protocol_active && is_letter {
			let ctrl_char = raw_ctrl_char(ch);
			return bytes.len() == 2 && bytes[0] == 0x1b && bytes[1] == ctrl_char;
		}

		if modifier == MOD_ALT && !kitty_protocol_active && is_letter {
			return bytes.len() == 2 && bytes[0] == 0x1b && bytes[1] == ch;
		}

		if modifier == MOD_CTRL {
			if is_letter {
				let raw = raw_ctrl_char(ch);
				if bytes.len() == 1 && bytes[0] == raw {
					return true;
				}
				return mok_matches(codepoint, MOD_CTRL) || kitty_matches(codepoint, MOD_CTRL);
			}
			if let Some(legacy_ctrl) = ctrl_symbol_to_byte(ch)
				&& bytes == [legacy_ctrl]
			{
				return true;
			}
			return mok_matches(codepoint, MOD_CTRL) || kitty_matches(codepoint, MOD_CTRL);
		}

		if modifier == (MOD_CTRL | MOD_SHIFT) {
			return kitty_matches(codepoint, MOD_SHIFT + MOD_CTRL)
				|| mok_matches(codepoint, MOD_SHIFT + MOD_CTRL);
		}

		if modifier == MOD_SHIFT {
			if is_letter && bytes.len() == 1 && bytes[0] == ch.to_ascii_uppercase() {
				return true;
			}
			return kitty_matches(codepoint, MOD_SHIFT) || mok_matches(codepoint, MOD_SHIFT);
		}

		if modifier != 0 {
			return kitty_matches(codepoint, modifier) || mok_matches(codepoint, modifier);
		}

		return (bytes.len() == 1 && bytes[0] == ch) || kitty_matches(codepoint, 0);
	}

	false
}

/// Match Kitty protocol input against a codepoint and modifier mask.
pub fn matches_kitty_sequence(
	bytes: &[u8],
	expected_codepoint: i32,
	expected_modifier: u32,
) -> bool {
	let Some(parsed) = parse_kitty_sequence(bytes) else {
		return false;
	};

	let actual_mod = parsed.modifier & !LOCK_MASK;
	let expected_mod = expected_modifier & !LOCK_MASK;
	if actual_mod != expected_mod {
		return false;
	}

	if parsed.codepoint == expected_codepoint {
		return true;
	}

	if let Some(base) = parsed.base_layout_key
		&& base == expected_codepoint
	{
		let cp = parsed.codepoint;
		let is_ascii_letter = u8::try_from(cp)
			.ok()
			.is_some_and(|b| b.is_ascii_alphabetic());
		let is_known_symbol = is_symbol_key(cp);
		if !is_ascii_letter && !is_known_symbol {
			return true;
		}
	}

	false
}

/// Check if input matches a legacy escape sequence for the given key name.
pub fn matches_legacy_sequence(bytes: &[u8], key_name: &str) -> bool {
	LEGACY_SEQUENCES
		.get(bytes)
		.is_some_and(|&id| id == key_name)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_matches_key_basic() {
		assert!(matches_key(b"\x03", "ctrl+c", false));
		assert!(matches_key(b"\x1b", "escape", false));
		assert!(matches_key(b"a", "a", false));
		assert!(matches_key(b" ", "space", false));
		assert!(matches_key(b"\t", "tab", false));
		assert!(matches_key(b"\r", "enter", false));
		assert!(matches_key(b"\x7f", "backspace", false));
	}

	#[test]
	fn test_matches_key_modifiers() {
		assert!(matches_key(b"\x1b\x7f", "alt+backspace", false));
		assert!(matches_key(b"\x1b\r", "alt+enter", false));
		assert!(matches_key(b"\x1b[Z", "shift+tab", false));
		assert!(matches_key(b"A", "shift+a", false));
	}

	#[test]
	fn test_matches_arrow_keys() {
		assert!(matches_key(b"\x1b[A", "up", false));
		assert!(matches_key(b"\x1b[B", "down", false));
		assert!(matches_key(b"\x1b[C", "right", false));
		assert!(matches_key(b"\x1b[D", "left", false));
	}

	#[test]
	fn test_matches_function_keys() {
		assert!(matches_key(b"\x1bOP", "f1", false));
		assert!(matches_key(b"\x1bOQ", "f2", false));
	}

	#[test]
	fn test_matches_kitty_sequence() {
		assert!(matches_kitty_sequence(b"\x1b[65;5u", 65, 4)); // ctrl+A
	}
}
