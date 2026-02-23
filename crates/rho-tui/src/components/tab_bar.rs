//! Tab bar component — horizontal tab navigation.
//!
//! Renders as: "Label:  Tab1   Tab2   Tab3  (tab to cycle)"
//! Navigation: Tab/Right = next, Shift+Tab/Left = previous (wraps).

use crate::component::{Component, InputResult};

/// Tab definition.
#[derive(Debug, Clone)]
pub struct Tab {
	pub id:    String,
	pub label: String,
}

impl Tab {
	pub fn new(id: &str, label: &str) -> Self {
		Self { id: id.to_owned(), label: label.to_owned() }
	}
}

/// Theme for styling the tab bar.
pub struct TabBarTheme {
	/// Style for the label prefix (e.g., "Settings:").
	pub label:        Box<dyn Fn(&str) -> String>,
	/// Style for the currently active tab.
	pub active_tab:   Box<dyn Fn(&str) -> String>,
	/// Style for inactive tabs.
	pub inactive_tab: Box<dyn Fn(&str) -> String>,
	/// Style for the hint text.
	pub hint:         Box<dyn Fn(&str) -> String>,
}

impl TabBarTheme {
	/// Create a plain (unstyled) theme.
	pub fn plain() -> Self {
		Self {
			label:        Box::new(|s| s.to_owned()),
			active_tab:   Box::new(|s| s.to_owned()),
			inactive_tab: Box::new(|s| s.to_owned()),
			hint:         Box::new(|s| s.to_owned()),
		}
	}
}

/// Callback type for tab change events.
pub type OnTabChange = Box<dyn FnMut(&Tab, usize)>;

/// Horizontal tab bar component.
pub struct TabBar {
	tabs:         Vec<Tab>,
	active_index: usize,
	theme:        TabBarTheme,
	label:        String,

	/// Called when the active tab changes.
	pub on_tab_change: Option<OnTabChange>,
}

impl TabBar {
	pub fn new(label: &str, tabs: Vec<Tab>, theme: TabBarTheme) -> Self {
		Self { tabs, active_index: 0, theme, label: label.to_owned(), on_tab_change: None }
	}

	pub fn with_initial_index(
		label: &str,
		tabs: Vec<Tab>,
		theme: TabBarTheme,
		initial_index: usize,
	) -> Self {
		Self {
			active_index: initial_index.min(tabs.len().saturating_sub(1)),
			tabs,
			theme,
			label: label.to_owned(),
			on_tab_change: None,
		}
	}

	pub fn active_tab(&self) -> Option<&Tab> {
		self.tabs.get(self.active_index)
	}

	pub const fn active_index(&self) -> usize {
		self.active_index
	}

	pub fn set_active_index(&mut self, index: usize) {
		if self.tabs.is_empty() {
			return;
		}
		let new_index = index.min(self.tabs.len() - 1);
		if new_index != self.active_index {
			self.active_index = new_index;
			if let Some(ref mut cb) = self.on_tab_change {
				cb(&self.tabs[self.active_index], self.active_index);
			}
		}
	}

	pub fn next_tab(&mut self) {
		if self.tabs.is_empty() {
			return;
		}
		let next = (self.active_index + 1) % self.tabs.len();
		self.set_active_index(next);
	}

	pub fn prev_tab(&mut self) {
		if self.tabs.is_empty() {
			return;
		}
		let prev = (self.active_index + self.tabs.len() - 1) % self.tabs.len();
		self.set_active_index(prev);
	}
}

impl Component for TabBar {
	fn render(&mut self, width: u16) -> Vec<String> {
		let mut parts = String::new();

		// Label prefix
		parts.push_str(&(self.theme.label)(&format!("{}:", self.label)));
		parts.push_str("  ");

		// Tab buttons
		for (i, tab) in self.tabs.iter().enumerate() {
			let padded_label = format!(" {} ", tab.label);
			if i == self.active_index {
				parts.push_str(&(self.theme.active_tab)(&padded_label));
			} else {
				parts.push_str(&(self.theme.inactive_tab)(&padded_label));
			}
			if i < self.tabs.len() - 1 {
				parts.push_str("  ");
			}
		}

		// Navigation hint
		parts.push_str("  ");
		parts.push_str(&(self.theme.hint)("(tab to cycle)"));

		let max_width = (width as usize).max(1);
		rho_text::wrap::wrap_text_with_ansi_str(&parts, max_width).into_vec()
	}

	fn handle_input(&mut self, data: &str) -> InputResult {
		let bytes = data.as_bytes();
		if crate::keys::match_key::matches_key(bytes, "tab", false)
			|| crate::keys::match_key::matches_key(bytes, "right", false)
		{
			self.next_tab();
			return InputResult::Consumed;
		}
		if crate::keys::match_key::matches_key(bytes, "shift+tab", false)
			|| crate::keys::match_key::matches_key(bytes, "left", false)
		{
			self.prev_tab();
			return InputResult::Consumed;
		}
		InputResult::Ignored
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_tabs() -> Vec<Tab> {
		vec![Tab::new("config", "Config"), Tab::new("tools", "Tools"), Tab::new("about", "About")]
	}

	#[test]
	fn test_tab_bar_render() {
		let mut bar = TabBar::new("Settings", make_tabs(), TabBarTheme::plain());
		let lines = bar.render(80);
		assert!(!lines.is_empty());
		let joined = lines.join("");
		assert!(joined.contains("Settings:"));
		assert!(joined.contains("Config"));
		assert!(joined.contains("Tools"));
		assert!(joined.contains("About"));
		assert!(joined.contains("(tab to cycle)"));
	}

	#[test]
	fn test_tab_bar_next_prev() {
		let mut bar = TabBar::new("Test", make_tabs(), TabBarTheme::plain());
		assert_eq!(bar.active_index(), 0);
		bar.next_tab();
		assert_eq!(bar.active_index(), 1);
		bar.next_tab();
		assert_eq!(bar.active_index(), 2);
		// Wrap around
		bar.next_tab();
		assert_eq!(bar.active_index(), 0);
		// Previous wraps
		bar.prev_tab();
		assert_eq!(bar.active_index(), 2);
	}

	#[test]
	fn test_tab_bar_input_tab_key() {
		let mut bar = TabBar::new("Test", make_tabs(), TabBarTheme::plain());
		assert_eq!(bar.active_index(), 0);
		let result = bar.handle_input("\t");
		assert_eq!(result, InputResult::Consumed);
		assert_eq!(bar.active_index(), 1);
	}

	#[test]
	fn test_tab_bar_input_ignored() {
		let mut bar = TabBar::new("Test", make_tabs(), TabBarTheme::plain());
		let result = bar.handle_input("a");
		assert_eq!(result, InputResult::Ignored);
		assert_eq!(bar.active_index(), 0);
	}

	#[test]
	fn test_tab_bar_callback() {
		let mut bar = TabBar::new("Test", make_tabs(), TabBarTheme::plain());
		bar.set_active_index(2);
		assert_eq!(bar.active_index(), 2);
		assert_eq!(bar.active_tab().unwrap().id, "about");
	}

	#[test]
	fn test_tab_bar_wrapping() {
		// Very narrow width should wrap
		let mut bar = TabBar::new("Settings", make_tabs(), TabBarTheme::plain());
		let lines = bar.render(20);
		assert!(lines.len() > 1);
	}
}
