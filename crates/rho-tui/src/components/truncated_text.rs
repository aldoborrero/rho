//! Truncated text component — single-line text that truncates to fit width.

use rho_text::truncate::EllipsisKind;

use super::text::make_padding;
use crate::component::Component;

/// Text component that truncates to fit viewport width.
///
/// Only renders the first line of text. Adds ellipsis when text is too long.
/// Horizontal padding is added without trailing spaces (avoids issues when
/// copying).
pub struct TruncatedText {
	text:      String,
	padding_x: usize,
	padding_y: usize,
}

impl TruncatedText {
	pub fn new(text: &str, padding_x: usize, padding_y: usize) -> Self {
		Self { text: text.to_owned(), padding_x, padding_y }
	}

	pub fn set_text(&mut self, text: &str) {
		text.clone_into(&mut self.text);
	}
}

impl Component for TruncatedText {
	fn render(&mut self, width: u16) -> Vec<String> {
		let w = width as usize;
		let mut result = Vec::new();

		// Empty line padded to width (for vertical padding)
		let empty_line = make_padding(w);

		// Top vertical padding
		for _ in 0..self.padding_y {
			result.push(empty_line.clone());
		}

		// Available width after horizontal padding
		let available_width = w.saturating_sub(self.padding_x * 2).max(1);

		// Take only first line
		let single_line = self
			.text
			.find('\n')
			.map_or(self.text.as_str(), |idx| &self.text[..idx]);

		// Truncate if needed
		let display_text = rho_text::truncate::truncate_to_width_str(
			single_line,
			available_width,
			EllipsisKind::Unicode,
			false,
		)
		.unwrap_or_else(|| single_line.to_owned());

		// Add horizontal padding (no trailing padding to full width)
		let left_pad = make_padding(self.padding_x);
		let right_pad = make_padding(self.padding_x);
		result.push(format!("{left_pad}{display_text}{right_pad}"));

		// Bottom vertical padding
		for _ in 0..self.padding_y {
			result.push(empty_line.clone());
		}

		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_horizontal_padding_no_trailing_spaces() {
		let mut text = TruncatedText::new("Hello world", 1, 0);
		let lines = text.render(50);
		assert_eq!(lines.len(), 1);
		// leftPad(1) + "Hello world"(11) + rightPad(1) = 13
		let vis = rho_text::width::visible_width_str(&lines[0]);
		assert_eq!(vis, 13);
	}

	#[test]
	fn test_vertical_padding() {
		let mut text = TruncatedText::new("Hello", 0, 2);
		let lines = text.render(40);
		// 2 top + 1 content + 2 bottom = 5
		assert_eq!(lines.len(), 5);
		// Vertical padding lines are full width
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 40);
		assert_eq!(rho_text::width::visible_width_str(&lines[1]), 40);
		assert_eq!(rho_text::width::visible_width_str(&lines[3]), 40);
		assert_eq!(rho_text::width::visible_width_str(&lines[4]), 40);
		// Content line is just the text
		assert_eq!(rho_text::width::visible_width_str(&lines[2]), 5);
	}

	#[test]
	fn test_truncates_long_text_with_ellipsis() {
		let long_text =
			"This is a very long piece of text that will definitely exceed the available width";
		let mut text = TruncatedText::new(long_text, 1, 0);
		let lines = text.render(30);
		assert_eq!(lines.len(), 1);
		// availableWidth = 30 - 2 = 28, truncated + padding = 30
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 30);
		// Should contain ellipsis
		let stripped = strip_ansi(&lines[0]);
		assert!(stripped.contains('\u{2026}')); // '…'
	}

	#[test]
	fn test_preserves_ansi_codes() {
		let styled = "\x1b[31mHello\x1b[0m \x1b[34mworld\x1b[0m";
		let mut text = TruncatedText::new(styled, 1, 0);
		let lines = text.render(40);
		assert_eq!(lines.len(), 1);
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 13);
		assert!(lines[0].contains("\x1b["));
	}

	#[test]
	fn test_truncates_styled_text_with_reset() {
		let styled = "\x1b[31mThis is a very long red text that will be truncated\x1b[0m";
		let mut text = TruncatedText::new(styled, 1, 0);
		let lines = text.render(20);
		assert_eq!(lines.len(), 1);
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 20);
		assert!(lines[0].contains("\x1b[0m\u{2026}"));
	}

	#[test]
	fn test_fits_without_truncation() {
		let mut text = TruncatedText::new("Hello world", 1, 0);
		let lines = text.render(30);
		assert_eq!(lines.len(), 1);
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 13);
		let stripped = strip_ansi(&lines[0]);
		assert!(!stripped.contains('\u{2026}'));
	}

	#[test]
	fn test_empty_text() {
		let mut text = TruncatedText::new("", 1, 0);
		let lines = text.render(30);
		assert_eq!(lines.len(), 1);
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 2);
	}

	#[test]
	fn test_stops_at_newline() {
		let multiline = "First line\nSecond line\nThird line";
		let mut text = TruncatedText::new(multiline, 1, 0);
		let lines = text.render(40);
		assert_eq!(lines.len(), 1);
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 12);
		let stripped = strip_ansi(&lines[0]).trim().to_owned();
		assert!(stripped.contains("First line"));
		assert!(!stripped.contains("Second line"));
	}

	#[test]
	fn test_truncates_first_line_with_newlines() {
		let mut text = TruncatedText::new(
			"This is a very long first line that needs truncation\nSecond line",
			1,
			0,
		);
		let lines = text.render(25);
		assert_eq!(lines.len(), 1);
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 25);
		let stripped = strip_ansi(&lines[0]);
		assert!(stripped.contains('\u{2026}'));
		assert!(!stripped.contains("Second line"));
	}

	fn strip_ansi(s: &str) -> String {
		let mut result = String::new();
		let mut in_seq = false;
		for c in s.chars() {
			if c == '\x1b' {
				in_seq = true;
			} else if in_seq {
				if c.is_ascii_alphabetic() {
					in_seq = false;
				}
			} else {
				result.push(c);
			}
		}
		result
	}
}
