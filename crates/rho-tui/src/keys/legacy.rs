//! Legacy terminal escape sequence map.
//!
//! Perfect hash map (PHF) for O(1) lookup of legacy escape sequences.

use phf::phf_map;

/// Perfect hash map for legacy sequences.
pub static LEGACY_SEQUENCES: phf::Map<&'static [u8], &'static str> = phf_map! {
	 // Arrow keys (SS3 and CSI)
	 b"\x1bOA" => "up", b"\x1bOB" => "down", b"\x1bOC" => "right", b"\x1bOD" => "left",
	 b"\x1b[A" => "up", b"\x1b[B" => "down", b"\x1b[C" => "right", b"\x1b[D" => "left",
	 // Home/End (multiple terminal variants)
	 b"\x1bOH" => "home", b"\x1bOF" => "end",
	 b"\x1b[H" => "home", b"\x1b[F" => "end",
	 b"\x1b[1~" => "home", b"\x1b[7~" => "home",
	 b"\x1b[4~" => "end", b"\x1b[8~" => "end",
	 // Clear
	 b"\x1b[E" => "clear", b"\x1bOE" => "clear", b"\x1bOe" => "ctrl+clear", b"\x1b[e" => "shift+clear",
	 // Insert/Delete
	 b"\x1b[2~" => "insert", b"\x1b[2$" => "shift+insert", b"\x1b[2^" => "ctrl+insert",
	 b"\x1b[3~" => "delete", b"\x1b[3$" => "shift+delete", b"\x1b[3^" => "ctrl+delete",
	 // Page Up/Down
	 b"\x1b[5~" => "pageUp", b"\x1b[6~" => "pageDown",
	 b"\x1b[[5~" => "pageUp", b"\x1b[[6~" => "pageDown",
	 // Shift+arrow
	 b"\x1b[a" => "shift+up", b"\x1b[b" => "shift+down", b"\x1b[c" => "shift+right", b"\x1b[d" => "shift+left",
	 // Ctrl+arrow
	 b"\x1bOa" => "ctrl+up", b"\x1bOb" => "ctrl+down", b"\x1bOc" => "ctrl+right", b"\x1bOd" => "ctrl+left",
	 // Shift+page/home/end
	 b"\x1b[5$" => "shift+pageUp", b"\x1b[6$" => "shift+pageDown",
	 b"\x1b[7$" => "shift+home", b"\x1b[8$" => "shift+end",
	 // Ctrl+page/home/end
	 b"\x1b[5^" => "ctrl+pageUp", b"\x1b[6^" => "ctrl+pageDown",
	 b"\x1b[7^" => "ctrl+home", b"\x1b[8^" => "ctrl+end",
	 // Function keys (SS3, CSI tilde, Linux console)
	 b"\x1bOP" => "f1", b"\x1bOQ" => "f2", b"\x1bOR" => "f3", b"\x1bOS" => "f4",
	 b"\x1b[11~" => "f1", b"\x1b[12~" => "f2", b"\x1b[13~" => "f3", b"\x1b[14~" => "f4",
	 b"\x1b[[A" => "f1", b"\x1b[[B" => "f2", b"\x1b[[C" => "f3", b"\x1b[[D" => "f4", b"\x1b[[E" => "f5",
	 b"\x1b[15~" => "f5", b"\x1b[17~" => "f6", b"\x1b[18~" => "f7", b"\x1b[19~" => "f8",
	 b"\x1b[20~" => "f9", b"\x1b[21~" => "f10", b"\x1b[23~" => "f11", b"\x1b[24~" => "f12",
	 // Alt+arrow (legacy)
	 b"\x1bb" => "alt+left", b"\x1bf" => "alt+right", b"\x1bp" => "alt+up", b"\x1bn" => "alt+down",
};

/// Check if bytes match a legacy key sequence.
pub fn matches_legacy_key(bytes: &[u8], key: &str) -> bool {
	LEGACY_SEQUENCES.get(bytes).is_some_and(|&id| id == key)
}

/// Check if bytes match a legacy modifier sequence (shift/ctrl variants).
pub fn matches_legacy_modifier_sequence(bytes: &[u8], key: &str, modifier: u32) -> bool {
	use super::types::{MOD_CTRL, MOD_SHIFT};

	if modifier == MOD_SHIFT {
		let expected = match key {
			"up" => Some("shift+up"),
			"down" => Some("shift+down"),
			"right" => Some("shift+right"),
			"left" => Some("shift+left"),
			"clear" => Some("shift+clear"),
			"insert" => Some("shift+insert"),
			"delete" => Some("shift+delete"),
			"pageUp" => Some("shift+pageUp"),
			"pageDown" => Some("shift+pageDown"),
			"home" => Some("shift+home"),
			"end" => Some("shift+end"),
			_ => None,
		};
		if let Some(expected_key) = expected {
			return LEGACY_SEQUENCES
				.get(bytes)
				.is_some_and(|&id| id == expected_key);
		}
	} else if modifier == MOD_CTRL {
		let expected = match key {
			"up" => Some("ctrl+up"),
			"down" => Some("ctrl+down"),
			"right" => Some("ctrl+right"),
			"left" => Some("ctrl+left"),
			"clear" => Some("ctrl+clear"),
			"insert" => Some("ctrl+insert"),
			"delete" => Some("ctrl+delete"),
			"pageUp" => Some("ctrl+pageUp"),
			"pageDown" => Some("ctrl+pageDown"),
			"home" => Some("ctrl+home"),
			"end" => Some("ctrl+end"),
			_ => None,
		};
		if let Some(expected_key) = expected {
			return LEGACY_SEQUENCES
				.get(bytes)
				.is_some_and(|&id| id == expected_key);
		}
	}
	false
}
