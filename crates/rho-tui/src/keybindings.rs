//! Editor keybinding management.
//!
//! Maps editor actions to key identifiers, with Emacs-style defaults.
//! Supports overriding bindings and shifted symbol normalization.

use std::{collections::HashMap, sync::OnceLock};

/// Editor actions that can be bound to keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorAction {
	// Cursor movement
	CursorUp,
	CursorDown,
	CursorLeft,
	CursorRight,
	CursorWordLeft,
	CursorWordRight,
	CursorLineStart,
	CursorLineEnd,
	JumpForward,
	JumpBackward,
	// Deletion
	DeleteCharBackward,
	DeleteCharForward,
	DeleteWordBackward,
	DeleteWordForward,
	DeleteToLineStart,
	DeleteToLineEnd,
	// Text input
	NewLine,
	Submit,
	Tab,
	// Selection/autocomplete
	SelectUp,
	SelectDown,
	SelectPageUp,
	SelectPageDown,
	SelectConfirm,
	SelectCancel,
	// Clipboard
	Copy,
	// Kill ring / undo
	Undo,
	Yank,
	YankPop,
}

/// All editor actions in declaration order.
pub const ALL_ACTIONS: &[EditorAction] = &[
	EditorAction::CursorUp,
	EditorAction::CursorDown,
	EditorAction::CursorLeft,
	EditorAction::CursorRight,
	EditorAction::CursorWordLeft,
	EditorAction::CursorWordRight,
	EditorAction::CursorLineStart,
	EditorAction::CursorLineEnd,
	EditorAction::JumpForward,
	EditorAction::JumpBackward,
	EditorAction::DeleteCharBackward,
	EditorAction::DeleteCharForward,
	EditorAction::DeleteWordBackward,
	EditorAction::DeleteWordForward,
	EditorAction::DeleteToLineStart,
	EditorAction::DeleteToLineEnd,
	EditorAction::NewLine,
	EditorAction::Submit,
	EditorAction::Tab,
	EditorAction::SelectUp,
	EditorAction::SelectDown,
	EditorAction::SelectPageUp,
	EditorAction::SelectPageDown,
	EditorAction::SelectConfirm,
	EditorAction::SelectCancel,
	EditorAction::Copy,
	EditorAction::Undo,
	EditorAction::Yank,
	EditorAction::YankPop,
];

/// A key identifier string (e.g. "ctrl+a", "up", "shift+enter").
pub type KeyId = String;

/// Default Emacs-style editor keybindings.
fn default_bindings() -> HashMap<EditorAction, Vec<KeyId>> {
	use EditorAction::*;
	let entries: &[(EditorAction, &[&str])] = &[
		(CursorUp, &["up"]),
		(CursorDown, &["down"]),
		(CursorLeft, &["left", "ctrl+b"]),
		(CursorRight, &["right", "ctrl+f"]),
		(CursorWordLeft, &["alt+left", "ctrl+left", "alt+b"]),
		(CursorWordRight, &["alt+right", "ctrl+right", "alt+f"]),
		(CursorLineStart, &["home", "ctrl+a"]),
		(CursorLineEnd, &["end", "ctrl+e"]),
		(JumpForward, &["ctrl+]"]),
		(JumpBackward, &["ctrl+alt+]"]),
		(DeleteCharBackward, &["backspace"]),
		(DeleteCharForward, &["delete", "ctrl+d"]),
		(DeleteWordBackward, &["ctrl+w", "alt+backspace", "ctrl+backspace"]),
		(DeleteWordForward, &["alt+delete", "alt+d"]),
		(DeleteToLineStart, &["ctrl+u"]),
		(DeleteToLineEnd, &["ctrl+k"]),
		(NewLine, &["shift+enter"]),
		(Submit, &["enter"]),
		(Tab, &["tab"]),
		(SelectUp, &["up"]),
		(SelectDown, &["down"]),
		(SelectPageUp, &["pageup"]),
		(SelectPageDown, &["pagedown"]),
		(SelectConfirm, &["enter"]),
		(SelectCancel, &["escape", "ctrl+c"]),
		(Copy, &["ctrl+c"]),
		(Undo, &["ctrl+-"]),
		(Yank, &["ctrl+y"]),
		(YankPop, &["alt+y"]),
	];

	let mut map = HashMap::new();
	for &(action, keys) in entries {
		map.insert(action, keys.iter().map(|k| (*k).to_owned()).collect());
	}
	map
}

/// Characters that are produced by Shift+key and should be matched without the
/// shift prefix.
const SHIFTED_SYMBOL_KEYS: &[char] = &[
	'!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+', '{', '}', '|', ':', '<', '>', '?',
	'~',
];

fn is_shifted_symbol(c: char) -> bool {
	SHIFTED_SYMBOL_KEYS.contains(&c)
}

/// Manages keybindings for the editor.
pub struct EditorKeybindingsManager {
	action_to_keys: HashMap<EditorAction, Vec<KeyId>>,
}

impl EditorKeybindingsManager {
	/// Create with default bindings.
	pub fn new() -> Self {
		Self { action_to_keys: default_bindings() }
	}

	/// Create with custom overrides on top of defaults.
	pub fn with_overrides(overrides: &HashMap<EditorAction, Vec<KeyId>>) -> Self {
		let mut bindings = default_bindings();
		for (action, keys) in overrides {
			bindings.insert(*action, keys.iter().map(|k| k.to_ascii_lowercase()).collect());
		}
		Self { action_to_keys: bindings }
	}

	/// Check if raw terminal input matches a specific action.
	pub fn matches(&self, data: &[u8], action: EditorAction, kitty_active: bool) -> bool {
		let Some(keys) = self.action_to_keys.get(&action) else {
			return false;
		};
		for key in keys {
			if crate::keys::match_key::matches_key(data, key, kitty_active) {
				return true;
			}
		}

		// Handle shifted symbols: if parsed key is "shift+X" where X is a symbol,
		// check if X itself is in the bindings
		if let Some(parsed) = crate::keys::parse::parse_key(data, kitty_active)
			&& let Some(rest) = parsed.strip_prefix("shift+")
			&& rest.len() == 1
			&& let Some(c) = rest.chars().next()
			&& is_shifted_symbol(c)
		{
			return keys.iter().any(|k| k == rest);
		}

		false
	}

	/// Get keys bound to an action.
	pub fn get_keys(&self, action: EditorAction) -> &[KeyId] {
		self.action_to_keys.get(&action).map_or(&[], Vec::as_slice)
	}

	/// Update configuration with overrides.
	pub fn set_config(&mut self, overrides: &HashMap<EditorAction, Vec<KeyId>>) {
		self.action_to_keys = default_bindings();
		for (action, keys) in overrides {
			self
				.action_to_keys
				.insert(*action, keys.iter().map(|k| k.to_ascii_lowercase()).collect());
		}
	}
}

impl Default for EditorKeybindingsManager {
	fn default() -> Self {
		Self::new()
	}
}

/// Global keybindings instance.
static GLOBAL_KEYBINDINGS: OnceLock<EditorKeybindingsManager> = OnceLock::new();

/// Get the global keybindings manager.
pub fn get_editor_keybindings() -> &'static EditorKeybindingsManager {
	GLOBAL_KEYBINDINGS.get_or_init(EditorKeybindingsManager::new)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_default_bindings_exist() {
		let mgr = EditorKeybindingsManager::new();
		for action in ALL_ACTIONS {
			assert!(!mgr.get_keys(*action).is_empty(), "action {action:?} has no bindings");
		}
	}

	#[test]
	fn test_matches_arrow_up() {
		let mgr = EditorKeybindingsManager::new();
		// ESC [ A is up arrow
		assert!(mgr.matches(b"\x1b[A", EditorAction::CursorUp, false));
	}

	#[test]
	fn test_matches_ctrl_a() {
		let mgr = EditorKeybindingsManager::new();
		// Ctrl+A is byte 0x01
		assert!(mgr.matches(b"\x01", EditorAction::CursorLineStart, false));
	}

	#[test]
	fn test_override_bindings() {
		let mut overrides = HashMap::new();
		overrides.insert(EditorAction::Submit, vec!["ctrl+m".to_owned()]);
		let mgr = EditorKeybindingsManager::with_overrides(&overrides);
		assert_eq!(mgr.get_keys(EditorAction::Submit), &["ctrl+m"]);
	}
}
