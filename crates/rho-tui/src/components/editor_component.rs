//! Interface for custom editor components.
//!
//! This allows extensions to provide their own editor implementation
//! (e.g., vim mode, emacs mode, custom keybindings) while maintaining
//! compatibility with the core application.

use super::editor::{AutocompleteProvider, BorderColorFn, TextCallback};
use crate::component::Focusable;

/// Interface for custom editor components.
///
/// Extends the base `Focusable` trait with editor-specific functionality
/// like text access, history, autocomplete, and paste expansion.
pub trait EditorComponent: Focusable {
	// ── Core text access (required) ────────────────────────────────

	/// Get the current text content.
	fn get_text(&self) -> String;

	/// Set the text content.
	fn set_text(&mut self, text: &str);

	// ── Callbacks ──────────────────────────────────────────────────

	/// Set the change callback. Called when the text content changes.
	fn set_on_change(&mut self, cb: TextCallback);

	// ── History support ────────────────────────────────────────────

	/// Add text to history for up/down navigation.
	fn add_to_history(&mut self, text: &str);

	// ── Advanced text manipulation ─────────────────────────────────

	/// Insert text at current cursor position.
	fn insert_text(&mut self, text: &str);

	/// Get text with any markers expanded (e.g., paste markers).
	/// Falls back to `get_text()` if not overridden.
	fn get_expanded_text(&self) -> String {
		self.get_text()
	}

	// ── Autocomplete support ───────────────────────────────────────

	/// Set the autocomplete provider.
	fn set_autocomplete_provider(&mut self, provider: Box<dyn AutocompleteProvider>);

	// ── Appearance ─────────────────────────────────────────────────

	/// Set the border color function.
	fn set_border_color(&mut self, color_fn: BorderColorFn);

	/// Set horizontal padding.
	fn set_padding_x(&mut self, padding: usize);
}
