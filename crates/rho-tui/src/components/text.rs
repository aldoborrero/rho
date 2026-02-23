//! Text component — multi-line text with word wrapping, padding, and
//! background.

use crate::component::Component;

/// Background function type — applies styling to a padded line.
pub type BgFn = Box<dyn Fn(&str) -> String>;

/// Text component that displays multi-line text with word wrapping.
pub struct Text {
	content:      String,
	padding_x:    usize,
	padding_y:    usize,
	custom_bg_fn: Option<BgFn>,

	// Render cache
	prev_content: Option<String>,
	cached_width: Option<u16>,
	cached_lines: Option<Vec<String>>,
}

impl Text {
	pub fn new(text: &str, padding_x: usize, padding_y: usize) -> Self {
		Self {
			content: text.to_owned(),
			padding_x,
			padding_y,
			custom_bg_fn: None,
			prev_content: None,
			cached_width: None,
			cached_lines: None,
		}
	}

	pub fn with_bg(text: &str, padding_x: usize, padding_y: usize, bg_fn: BgFn) -> Self {
		Self {
			content: text.to_owned(),
			padding_x,
			padding_y,
			custom_bg_fn: Some(bg_fn),
			prev_content: None,
			cached_width: None,
			cached_lines: None,
		}
	}

	pub fn text(&self) -> &str {
		&self.content
	}

	pub fn set_text(&mut self, text: &str) {
		text.clone_into(&mut self.content);
		self.invalidate_cache();
	}

	pub fn set_custom_bg_fn(&mut self, bg_fn: Option<BgFn>) {
		self.custom_bg_fn = bg_fn;
		self.invalidate_cache();
	}

	fn invalidate_cache(&mut self) {
		self.prev_content = None;
		self.cached_width = None;
		self.cached_lines = None;
	}
}

impl Default for Text {
	fn default() -> Self {
		Self::new("", 1, 1)
	}
}

impl Component for Text {
	fn render(&mut self, width: u16) -> Vec<String> {
		// Check cache
		if let Some(ref cached) = self.cached_lines
			&& self.prev_content.as_deref() == Some(&self.content)
			&& self.cached_width == Some(width)
		{
			return cached.clone();
		}

		// Empty text → empty result
		if self.content.is_empty() || self.content.trim().is_empty() {
			return Vec::new();
		}

		let w = width as usize;

		// Replace tabs with 3 spaces
		let normalized = self.content.replace('\t', "   ");

		// Content width after padding
		let content_width = w.saturating_sub(self.padding_x * 2).max(1);

		// Wrap text
		let wrapped = rho_text::wrap::wrap_text_with_ansi_str(&normalized, content_width);

		// Build content lines with padding
		let left_pad = make_padding(self.padding_x);
		let right_pad = make_padding(self.padding_x);
		let mut content_lines = Vec::with_capacity(wrapped.len());

		for line in &wrapped {
			let with_margins = format!("{left_pad}{line}{right_pad}");

			if let Some(ref bg_fn) = self.custom_bg_fn {
				content_lines.push(apply_background_to_line(&with_margins, w, bg_fn));
			} else {
				let vis_len = rho_text::width::visible_width_str(&with_margins);
				let pad_needed = w.saturating_sub(vis_len);
				let mut padded = with_margins;
				pad_spaces(&mut padded, pad_needed);
				content_lines.push(padded);
			}
		}

		// Vertical padding (empty lines)
		let empty = make_padding(w);
		let empty_line = if let Some(ref bg_fn) = self.custom_bg_fn {
			apply_background_to_line(&empty, w, bg_fn)
		} else {
			empty
		};

		let mut result = Vec::with_capacity(self.padding_y * 2 + content_lines.len());
		for _ in 0..self.padding_y {
			result.push(empty_line.clone());
		}
		result.extend(content_lines);
		for _ in 0..self.padding_y {
			result.push(empty_line.clone());
		}

		if result.is_empty() {
			vec![String::new()]
		} else {
			result
		}
	}

	fn invalidate(&mut self) {
		self.invalidate_cache();
	}
}

/// Create a string of N spaces.
pub fn make_padding(n: usize) -> String {
	" ".repeat(n)
}

/// Pad a string with N trailing spaces.
fn pad_spaces(s: &mut String, n: usize) {
	for _ in 0..n {
		s.push(' ');
	}
}

/// Apply background function to a line, padding to full width first.
pub fn apply_background_to_line(
	line: &str,
	width: usize,
	bg_fn: &dyn Fn(&str) -> String,
) -> String {
	let vis_len = rho_text::width::visible_width_str(line);
	let pad_needed = width.saturating_sub(vis_len);
	let mut padded = line.to_owned();
	pad_spaces(&mut padded, pad_needed);
	bg_fn(&padded)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_text_empty() {
		let mut text = Text::new("", 1, 1);
		let lines = text.render(80);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_text_simple() {
		let mut text = Text::new("Hello world", 1, 0);
		let lines = text.render(80);
		assert_eq!(lines.len(), 1);
		// Should contain "Hello world" with padding
		assert!(lines[0].contains("Hello world"));
		// Visible width should be 80 (padded to full width)
		let w = rho_text::width::visible_width_str(&lines[0]);
		assert_eq!(w, 80);
	}

	#[test]
	fn test_text_with_vertical_padding() {
		let mut text = Text::new("Hello", 0, 2);
		let lines = text.render(40);
		// 2 top padding + 1 content + 2 bottom padding = 5
		assert_eq!(lines.len(), 5);
	}

	#[test]
	fn test_text_wrapping() {
		let mut text = Text::new("hello world this is a test", 0, 0);
		let lines = text.render(12);
		assert!(lines.len() > 1);
		for line in &lines {
			assert!(rho_text::width::visible_width_str(line) <= 12);
		}
	}

	#[test]
	fn test_text_with_background() {
		let mut text = Text::with_bg("Hi", 1, 0, Box::new(|s| format!("\x1b[44m{s}\x1b[0m")));
		let lines = text.render(20);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].contains("\x1b[44m"));
	}

	#[test]
	fn test_text_tab_replacement() {
		let mut text = Text::new("a\tb", 0, 0);
		let lines = text.render(80);
		assert_eq!(lines.len(), 1);
		// Tab replaced with 3 spaces: "a   b" = 5 visible chars
		assert!(lines[0].starts_with("a   b"));
	}
}
