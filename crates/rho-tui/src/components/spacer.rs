//! Spacer component — renders empty lines.

use crate::component::Component;

/// Spacer component that renders N empty lines.
pub struct Spacer {
	lines: usize,
}

impl Spacer {
	pub const fn new(lines: usize) -> Self {
		Self { lines }
	}

	pub const fn set_lines(&mut self, lines: usize) {
		self.lines = lines;
	}
}

impl Default for Spacer {
	fn default() -> Self {
		Self::new(1)
	}
}

impl Component for Spacer {
	fn render(&mut self, _width: u16) -> Vec<String> {
		vec![String::new(); self.lines]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_spacer_default() {
		let mut spacer = Spacer::default();
		let lines = spacer.render(80);
		assert_eq!(lines.len(), 1);
		assert_eq!(lines[0], "");
	}

	#[test]
	fn test_spacer_multiple_lines() {
		let mut spacer = Spacer::new(3);
		let lines = spacer.render(80);
		assert_eq!(lines.len(), 3);
		assert!(lines.iter().all(String::is_empty));
	}

	#[test]
	fn test_spacer_zero_lines() {
		let mut spacer = Spacer::new(0);
		let lines = spacer.render(80);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_spacer_set_lines() {
		let mut spacer = Spacer::new(1);
		spacer.set_lines(5);
		let lines = spacer.render(80);
		assert_eq!(lines.len(), 5);
	}
}
