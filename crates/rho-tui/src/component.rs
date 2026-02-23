//! Component traits and container for the TUI framework.
//!
//! Components produce ANSI-escaped string lines — the same rendering model
//! as the TypeScript version.

/// Cursor position marker — APC (Application Program Command) sequence.
///
/// This is a zero-width escape sequence that terminals ignore.
/// Components emit this at the cursor position when focused.
/// TUI finds and strips this marker, then positions the hardware cursor there.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// Result of input handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputResult {
	/// Input was consumed by the component.
	Consumed,
	/// Input was not handled — pass to next handler.
	Ignored,
	/// The component submitted text (e.g. editor Enter key).
	Submit(String),
}

/// Component interface — all UI components must implement this.
pub trait Component {
	/// Render the component to lines for the given viewport width.
	fn render(&mut self, width: u16) -> Vec<String>;

	/// Handle keyboard input when component has focus.
	/// Returns `InputResult::Consumed` if the input was handled.
	fn handle_input(&mut self, _data: &str) -> InputResult {
		InputResult::Ignored
	}

	/// If true, component receives key release events (Kitty protocol).
	/// Default is false — release events are filtered out.
	fn wants_key_release(&self) -> bool {
		false
	}

	/// Invalidate any cached rendering state.
	/// Called when theme changes or when component needs to re-render from
	/// scratch.
	fn invalidate(&mut self) {}
}

/// Interface for components that can receive focus and display a hardware
/// cursor.
///
/// When focused, the component should emit `CURSOR_MARKER` at the cursor
/// position in its render output. TUI will find this marker and position the
/// hardware cursor there for proper IME candidate window positioning.
pub trait Focusable: Component {
	fn set_focused(&mut self, focused: bool);
	fn is_focused(&self) -> bool;
}

/// Container — a component that contains other components.
pub struct Container {
	children: Vec<Box<dyn Component>>,
}

impl Container {
	pub const fn new() -> Self {
		Self { children: Vec::new() }
	}

	pub fn add_child(&mut self, component: Box<dyn Component>) {
		self.children.push(component);
	}

	pub fn remove_child(&mut self, index: usize) {
		if index < self.children.len() {
			self.children.remove(index);
		}
	}

	pub fn clear(&mut self) {
		self.children.clear();
	}

	pub fn children(&self) -> &[Box<dyn Component>] {
		&self.children
	}

	pub fn children_mut(&mut self) -> &mut [Box<dyn Component>] {
		&mut self.children
	}
}

impl Default for Container {
	fn default() -> Self {
		Self::new()
	}
}

impl Component for Container {
	fn render(&mut self, width: u16) -> Vec<String> {
		let mut lines = Vec::new();
		for child in &mut self.children {
			lines.extend(child.render(width));
		}
		lines
	}

	fn invalidate(&mut self) {
		for child in &mut self.children {
			child.invalidate();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct TestComponent {
		lines: Vec<String>,
	}

	impl TestComponent {
		fn new(lines: &[&str]) -> Self {
			Self { lines: lines.iter().map(|s| (*s).to_owned()).collect() }
		}
	}

	impl Component for TestComponent {
		fn render(&mut self, _width: u16) -> Vec<String> {
			self.lines.clone()
		}
	}

	#[test]
	fn test_container_render() {
		let mut container = Container::new();
		container.add_child(Box::new(TestComponent::new(&["line1", "line2"])));
		container.add_child(Box::new(TestComponent::new(&["line3"])));

		let lines = container.render(80);
		assert_eq!(lines, vec!["line1", "line2", "line3"]);
	}

	#[test]
	fn test_container_clear() {
		let mut container = Container::new();
		container.add_child(Box::new(TestComponent::new(&["a"])));
		assert_eq!(container.children().len(), 1);
		container.clear();
		assert!(container.children().is_empty());
	}

	#[test]
	fn test_cursor_marker() {
		assert!(CURSOR_MARKER.starts_with('\x1b'));
		assert!(CURSOR_MARKER.ends_with('\x07'));
	}
}
