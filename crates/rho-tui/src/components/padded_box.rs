//! Padded box component — container with padding and background for child
//! components.

use super::text::{BgFn, apply_background_to_line, make_padding};
use crate::component::Component;

/// Padded box component — applies padding and background to all children.
pub struct PaddedBox {
	children:  Vec<Box<dyn Component>>,
	padding_x: usize,
	padding_y: usize,
	bg_fn:     Option<BgFn>,

	// Simple cache: store hash of child output + result
	cached_key:    Option<u64>,
	cached_result: Option<Vec<String>>,
}

impl PaddedBox {
	pub fn new(padding_x: usize, padding_y: usize) -> Self {
		Self {
			children: Vec::new(),
			padding_x,
			padding_y,
			bg_fn: None,
			cached_key: None,
			cached_result: None,
		}
	}

	pub fn with_bg(padding_x: usize, padding_y: usize, bg_fn: BgFn) -> Self {
		Self {
			children: Vec::new(),
			padding_x,
			padding_y,
			bg_fn: Some(bg_fn),
			cached_key: None,
			cached_result: None,
		}
	}

	pub fn add_child(&mut self, component: Box<dyn Component>) {
		self.children.push(component);
		self.invalidate_cache();
	}

	pub fn remove_child(&mut self, index: usize) {
		if index < self.children.len() {
			self.children.remove(index);
			self.invalidate_cache();
		}
	}

	pub fn clear(&mut self) {
		self.children.clear();
		self.invalidate_cache();
	}

	pub fn set_bg_fn(&mut self, bg_fn: Option<BgFn>) {
		self.bg_fn = bg_fn;
	}

	pub fn children(&self) -> &[Box<dyn Component>] {
		&self.children
	}

	pub fn children_mut(&mut self) -> &mut [Box<dyn Component>] {
		&mut self.children
	}

	fn invalidate_cache(&mut self) {
		self.cached_key = None;
		self.cached_result = None;
	}

	/// Simple hash of width + child line contents for cache validation.
	fn compute_cache_key(width: u16, child_lines: &[String], bg_sample: Option<&str>) -> u64 {
		// Simple FNV-1a hash
		let mut h: u64 = 0xcbf2_9ce4_8422_2325;
		let mix = |h: &mut u64, bytes: &[u8]| {
			for &b in bytes {
				*h ^= u64::from(b);
				*h = h.wrapping_mul(0x0100_0000_01b3);
			}
		};
		mix(&mut h, &width.to_le_bytes());
		mix(&mut h, &(child_lines.len() as u64).to_le_bytes());
		for line in child_lines {
			mix(&mut h, &(line.len() as u64).to_le_bytes());
			mix(&mut h, line.as_bytes());
		}
		if let Some(sample) = bg_sample {
			mix(&mut h, sample.as_bytes());
		}
		h
	}

	fn apply_bg(&self, line: &str, width: usize) -> String {
		let vis_len = rho_text::width::visible_width_str(line);
		let pad_needed = width.saturating_sub(vis_len);
		let mut padded = line.to_owned();
		for _ in 0..pad_needed {
			padded.push(' ');
		}
		if let Some(ref bg_fn) = self.bg_fn {
			apply_background_to_line(&padded, width, bg_fn)
		} else {
			padded
		}
	}
}

impl Component for PaddedBox {
	fn render(&mut self, width: u16) -> Vec<String> {
		if self.children.is_empty() {
			return Vec::new();
		}

		let w = width as usize;
		let content_width = w.saturating_sub(self.padding_x * 2).max(1);
		let left_pad = make_padding(self.padding_x);

		// Render all children
		let mut child_lines = Vec::new();
		for child in &mut self.children {
			for line in child.render(content_width as u16) {
				child_lines.push(format!("{left_pad}{line}"));
			}
		}

		if child_lines.is_empty() {
			return Vec::new();
		}

		// Check bg sample for cache validation
		let bg_sample = self.bg_fn.as_ref().map(|f| f("test"));
		let cache_key = Self::compute_cache_key(width, &child_lines, bg_sample.as_deref());

		if self.cached_key == Some(cache_key)
			&& let Some(ref cached) = self.cached_result
		{
			return cached.clone();
		}

		// Build result with padding
		let mut result = Vec::with_capacity(self.padding_y * 2 + child_lines.len());

		for _ in 0..self.padding_y {
			result.push(self.apply_bg("", w));
		}
		for line in &child_lines {
			result.push(self.apply_bg(line, w));
		}
		for _ in 0..self.padding_y {
			result.push(self.apply_bg("", w));
		}

		result
	}

	fn invalidate(&mut self) {
		self.invalidate_cache();
		for child in &mut self.children {
			child.invalidate();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct FakeComponent {
		lines: Vec<String>,
	}

	impl FakeComponent {
		fn new(lines: &[&str]) -> Self {
			Self { lines: lines.iter().map(|s| (*s).to_owned()).collect() }
		}
	}

	impl Component for FakeComponent {
		fn render(&mut self, _width: u16) -> Vec<String> {
			self.lines.clone()
		}
	}

	#[test]
	fn test_box_empty() {
		let mut bx = PaddedBox::new(1, 1);
		let lines = bx.render(40);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_box_with_child() {
		let mut bx = PaddedBox::new(1, 0);
		bx.add_child(Box::new(FakeComponent::new(&["hello"])));
		let lines = bx.render(40);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].contains("hello"));
		assert_eq!(rho_text::width::visible_width_str(&lines[0]), 40);
	}

	#[test]
	fn test_box_with_padding() {
		let mut bx = PaddedBox::new(2, 1);
		bx.add_child(Box::new(FakeComponent::new(&["hi"])));
		let lines = bx.render(30);
		// 1 top pad + 1 content + 1 bottom pad = 3
		assert_eq!(lines.len(), 3);
		for line in &lines {
			assert_eq!(rho_text::width::visible_width_str(line), 30);
		}
	}

	#[test]
	fn test_box_with_background() {
		let mut bx = PaddedBox::with_bg(0, 0, Box::new(|s| format!("\x1b[44m{s}\x1b[0m")));
		bx.add_child(Box::new(FakeComponent::new(&["test"])));
		let lines = bx.render(20);
		assert_eq!(lines.len(), 1);
		assert!(lines[0].contains("\x1b[44m"));
	}

	#[test]
	fn test_box_clear() {
		let mut bx = PaddedBox::new(0, 0);
		bx.add_child(Box::new(FakeComponent::new(&["a"])));
		assert_eq!(bx.children().len(), 1);
		bx.clear();
		assert!(bx.children().is_empty());
	}
}
