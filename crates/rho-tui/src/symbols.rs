//! Symbol and box drawing character types for TUI rendering.

/// Box drawing characters.
#[derive(Debug, Clone)]
pub struct BoxSymbols {
	pub top_left:     &'static str,
	pub top_right:    &'static str,
	pub bottom_left:  &'static str,
	pub bottom_right: &'static str,
	pub horizontal:   &'static str,
	pub vertical:     &'static str,
	pub tee_down:     &'static str,
	pub tee_up:       &'static str,
	pub tee_left:     &'static str,
	pub tee_right:    &'static str,
	pub cross:        &'static str,
}

/// Rounded box symbols (subset without tee/cross).
#[derive(Debug, Clone)]
pub struct RoundedBoxSymbols {
	pub top_left:     &'static str,
	pub top_right:    &'static str,
	pub bottom_left:  &'static str,
	pub bottom_right: &'static str,
	pub horizontal:   &'static str,
	pub vertical:     &'static str,
}

/// Complete symbol theme for the TUI.
#[derive(Debug, Clone)]
pub struct SymbolTheme {
	pub cursor:         &'static str,
	pub input_cursor:   &'static str,
	pub box_round:      RoundedBoxSymbols,
	pub box_sharp:      BoxSymbols,
	pub table:          BoxSymbols,
	pub quote_border:   &'static str,
	pub hr_char:        &'static str,
	pub spinner_frames: &'static [&'static str],
}
