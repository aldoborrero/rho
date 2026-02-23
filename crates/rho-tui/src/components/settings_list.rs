//! Settings list component — settings panel with value cycling and submenu
//! support.

use std::cmp::{max, min};

use rho_text::truncate::EllipsisKind;

use super::text::make_padding;
use crate::component::{Component, InputResult};

/// Style function for label/value with selected state.
pub type StyledFn = Box<dyn Fn(&str, bool) -> String>;

/// Callback type for setting change events.
pub type OnSettingChange = Box<dyn FnMut(&str, &str)>;

/// A single setting item.
#[derive(Debug, Clone)]
pub struct SettingItem {
	pub id:            String,
	pub label:         String,
	pub description:   Option<String>,
	pub current_value: String,
	/// If provided, Enter/Space cycles through these values.
	pub values:        Option<Vec<String>>,
}

impl SettingItem {
	pub fn new(id: &str, label: &str, current_value: &str) -> Self {
		Self {
			id:            id.to_owned(),
			label:         label.to_owned(),
			description:   None,
			current_value: current_value.to_owned(),
			values:        None,
		}
	}

	pub fn with_values(mut self, values: Vec<String>) -> Self {
		self.values = Some(values);
		self
	}

	pub fn with_description(mut self, desc: &str) -> Self {
		self.description = Some(desc.to_owned());
		self
	}
}

/// Theme for styling the settings list.
pub struct SettingsListTheme {
	pub label:       StyledFn,
	pub value:       StyledFn,
	pub description: Box<dyn Fn(&str) -> String>,
	pub cursor:      String,
	pub hint:        Box<dyn Fn(&str) -> String>,
}

/// Settings list component.
pub struct SettingsList {
	items:          Vec<SettingItem>,
	theme:          SettingsListTheme,
	selected_index: usize,
	max_visible:    usize,
	on_change:      OnSettingChange,
	on_cancel:      Box<dyn FnMut()>,
}

impl SettingsList {
	pub fn new(
		items: Vec<SettingItem>,
		max_visible: usize,
		theme: SettingsListTheme,
		on_change: OnSettingChange,
		on_cancel: Box<dyn FnMut()>,
	) -> Self {
		Self { items, theme, selected_index: 0, max_visible, on_change, on_cancel }
	}

	pub fn update_value(&mut self, id: &str, new_value: &str) {
		if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
			new_value.clone_into(&mut item.current_value);
		}
	}

	fn truncate_no_ellipsis(text: &str, max_width: usize) -> String {
		rho_text::truncate::truncate_to_width_str(text, max_width, EllipsisKind::None, false)
			.unwrap_or_else(|| text.to_owned())
	}

	fn activate_item(&mut self) {
		let Some(item) = self.items.get_mut(self.selected_index) else {
			return;
		};

		if let Some(ref values) = item.values
			&& !values.is_empty()
		{
			let current_idx = values
				.iter()
				.position(|v| v == &item.current_value)
				.unwrap_or(0);
			let next_idx = (current_idx + 1) % values.len();
			let new_value = values[next_idx].clone();
			item.current_value.clone_from(&new_value);
			(self.on_change)(&item.id, &new_value);
		}
	}
}

impl Component for SettingsList {
	fn render(&mut self, width: u16) -> Vec<String> {
		let w = width as usize;
		let mut lines = Vec::new();

		if self.items.is_empty() {
			lines.push((self.theme.hint)("  No settings available"));
			return lines;
		}

		// Visible range
		let total = self.items.len();
		let start = max(
			0,
			min(
				self.selected_index.saturating_sub(self.max_visible / 2),
				total.saturating_sub(self.max_visible),
			),
		);
		let end = min(start + self.max_visible, total);

		// Max label width for alignment
		let max_label_width = min(
			30,
			self
				.items
				.iter()
				.map(|item| rho_text::width::visible_width_str(&item.label))
				.max()
				.unwrap_or(0),
		);

		for i in start..end {
			let item = &self.items[i];
			let is_selected = i == self.selected_index;
			let prefix = if is_selected {
				&self.theme.cursor
			} else {
				"  "
			};
			let prefix_width = rho_text::width::visible_width_str(prefix);

			// Pad label
			let label_vis_width = rho_text::width::visible_width_str(&item.label);
			let label_padded = format!(
				"{}{}",
				item.label,
				make_padding(max_label_width.saturating_sub(label_vis_width))
			);
			let label_text = (self.theme.label)(&label_padded, is_selected);

			let separator = "  ";
			let used_width = prefix_width + max_label_width + separator.len();
			let value_max = w.saturating_sub(used_width + 2);
			let value_text = (self.theme.value)(
				&Self::truncate_no_ellipsis(&item.current_value, value_max),
				is_selected,
			);

			let full = format!("{prefix}{label_text}{separator}{value_text}");
			lines.push(Self::truncate_no_ellipsis(&full, w));
		}

		// Scroll indicator
		if start > 0 || end < total {
			let scroll_text = format!("  ({}/{})", self.selected_index + 1, total);
			lines.push((self.theme.hint)(&Self::truncate_no_ellipsis(
				&scroll_text,
				w.saturating_sub(2),
			)));
		}

		// Description for selected item
		if let Some(item) = self.items.get(self.selected_index)
			&& let Some(ref desc) = item.description
		{
			lines.push(String::new());
			let wrapped = rho_text::wrap::wrap_text_with_ansi_str(desc, w.saturating_sub(4));
			for line in &wrapped {
				lines.push((self.theme.description)(&format!("  {line}")));
			}
		}

		// Hint
		lines.push(String::new());
		lines.push(Self::truncate_no_ellipsis(
			&(self.theme.hint)("  Enter/Space to change \u{00b7} Esc to cancel"),
			w,
		));

		lines
	}

	fn handle_input(&mut self, data: &str) -> InputResult {
		let bytes = data.as_bytes();

		if crate::keys::match_key::matches_key(bytes, "up", false) {
			let total = self.items.len();
			self.selected_index = if self.selected_index == 0 {
				total - 1
			} else {
				self.selected_index - 1
			};
			return InputResult::Consumed;
		}

		if crate::keys::match_key::matches_key(bytes, "down", false) {
			let total = self.items.len();
			self.selected_index = if self.selected_index >= total - 1 {
				0
			} else {
				self.selected_index + 1
			};
			return InputResult::Consumed;
		}

		if crate::keys::match_key::matches_key(bytes, "enter", false)
			|| crate::keys::match_key::matches_key(bytes, "return", false)
			|| data == "\n"
			|| data == " "
		{
			self.activate_item();
			return InputResult::Consumed;
		}

		if crate::keys::match_key::matches_key(bytes, "escape", false)
			|| crate::keys::match_key::matches_key(bytes, "esc", false)
			|| crate::keys::match_key::matches_key(bytes, "ctrl+c", false)
		{
			(self.on_cancel)();
			return InputResult::Consumed;
		}

		InputResult::Ignored
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plain_theme() -> SettingsListTheme {
		SettingsListTheme {
			label:       Box::new(|s, _| s.to_owned()),
			value:       Box::new(|s, _| s.to_owned()),
			description: Box::new(|s| s.to_owned()),
			cursor:      "> ".to_owned(),
			hint:        Box::new(|s| s.to_owned()),
		}
	}

	fn make_items() -> Vec<SettingItem> {
		vec![
			SettingItem::new("theme", "Theme", "dark").with_values(vec![
				"dark".into(),
				"light".into(),
				"auto".into(),
			]),
			SettingItem::new("font", "Font Size", "14"),
			SettingItem::new("wrap", "Word Wrap", "on")
				.with_description("Enable word wrapping in the editor"),
		]
	}

	#[test]
	fn test_settings_list_render() {
		let mut list =
			SettingsList::new(make_items(), 10, plain_theme(), Box::new(|_, _| {}), Box::new(|| {}));
		let lines = list.render(60);
		assert!(lines.len() >= 3);
		// Should contain item labels
		let joined = lines.join("\n");
		assert!(joined.contains("Theme"));
		assert!(joined.contains("Font Size"));
	}

	#[test]
	fn test_settings_list_nav() {
		let mut list =
			SettingsList::new(make_items(), 10, plain_theme(), Box::new(|_, _| {}), Box::new(|| {}));
		assert_eq!(list.selected_index, 0);
		list.handle_input("\x1b[B"); // down
		assert_eq!(list.selected_index, 1);
		list.handle_input("\x1b[A"); // up
		assert_eq!(list.selected_index, 0);
	}

	#[test]
	fn test_settings_list_cycle_value() {
		let mut last_change: Option<(String, String)> = None;
		let change_ref = &mut last_change as *mut Option<(String, String)>;
		let mut list = SettingsList::new(
			make_items(),
			10,
			plain_theme(),
			Box::new(move |id, val| unsafe {
				*change_ref = Some((id.to_owned(), val.to_owned()));
			}),
			Box::new(|| {}),
		);
		// Activate first item (Theme: dark → light)
		list.handle_input("\r"); // enter
		assert_eq!(list.items[0].current_value, "light");
	}

	#[test]
	fn test_settings_list_description() {
		let mut list =
			SettingsList::new(make_items(), 10, plain_theme(), Box::new(|_, _| {}), Box::new(|| {}));
		// Navigate to "wrap" which has description
		list.handle_input("\x1b[B"); // down
		list.handle_input("\x1b[B"); // down
		let lines = list.render(60);
		let joined = lines.join("\n");
		assert!(joined.contains("word wrapping"));
	}
}
