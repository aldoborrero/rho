//! Select list component — scrollable list with arrow navigation and filtering.

use std::cmp::{max, min};

use rho_text::truncate::EllipsisKind;

use super::text::make_padding;
use crate::{
	component::{Component, InputResult},
	symbols::SymbolTheme,
};

/// Callback type for select events.
pub type OnSelect = Box<dyn FnMut(&SelectItem)>;

/// Item in a select list.
#[derive(Debug, Clone)]
pub struct SelectItem {
	pub value:       String,
	pub label:       String,
	pub description: Option<String>,
	/// Dim hint text shown inline when this item is selected.
	pub hint:        Option<String>,
}

impl SelectItem {
	pub fn new(value: &str, label: &str) -> Self {
		Self {
			value:       value.to_owned(),
			label:       label.to_owned(),
			description: None,
			hint:        None,
		}
	}

	pub fn with_description(mut self, desc: &str) -> Self {
		self.description = Some(desc.to_owned());
		self
	}
}

/// Theme for styling the select list.
pub struct SelectListTheme {
	pub selected_prefix: Box<dyn Fn(&str) -> String>,
	pub selected_text:   Box<dyn Fn(&str) -> String>,
	pub description:     Box<dyn Fn(&str) -> String>,
	pub scroll_info:     Box<dyn Fn(&str) -> String>,
	pub no_match:        Box<dyn Fn(&str) -> String>,
	pub symbols:         SymbolTheme,
}

/// Scrollable select list with arrow navigation.
pub struct SelectList {
	items:          Vec<SelectItem>,
	filtered_items: Vec<usize>, // indices into items
	selected_index: usize,
	max_visible:    usize,
	theme:          SelectListTheme,

	pub on_select:           Option<OnSelect>,
	pub on_cancel:           Option<Box<dyn FnMut()>>,
	pub on_selection_change: Option<OnSelect>,
}

impl SelectList {
	pub fn new(items: Vec<SelectItem>, max_visible: usize, theme: SelectListTheme) -> Self {
		let filtered_items: Vec<usize> = (0..items.len()).collect();
		Self {
			items,
			filtered_items,
			selected_index: 0,
			max_visible,
			theme,
			on_select: None,
			on_cancel: None,
			on_selection_change: None,
		}
	}

	pub fn set_filter(&mut self, filter: &str) {
		let filter_lower = filter.to_lowercase();
		self.filtered_items = self
			.items
			.iter()
			.enumerate()
			.filter(|(_, item)| item.value.to_lowercase().starts_with(&filter_lower))
			.map(|(i, _)| i)
			.collect();
		self.selected_index = 0;
	}

	pub fn set_selected_index(&mut self, index: usize) {
		if !self.filtered_items.is_empty() {
			self.selected_index = index.min(self.filtered_items.len() - 1);
		}
	}

	pub fn selected_item(&self) -> Option<&SelectItem> {
		self
			.filtered_items
			.get(self.selected_index)
			.map(|&idx| &self.items[idx])
	}

	fn notify_selection_change(&mut self) {
		if let Some(ref mut cb) = self.on_selection_change
			&& let Some(&idx) = self.filtered_items.get(self.selected_index)
		{
			cb(&self.items[idx]);
		}
	}

	fn truncate_no_ellipsis(text: &str, max_width: usize) -> String {
		rho_text::truncate::truncate_to_width_str(text, max_width, EllipsisKind::None, false)
			.unwrap_or_else(|| text.to_owned())
	}
}

impl Component for SelectList {
	fn render(&mut self, width: u16) -> Vec<String> {
		let w = width as usize;
		let mut lines = Vec::new();

		if self.filtered_items.is_empty() {
			lines.push((self.theme.no_match)("  No matching commands"));
			return lines;
		}

		// Visible range with scrolling
		let total = self.filtered_items.len();
		let start = max(
			0,
			min(
				self.selected_index.saturating_sub(self.max_visible / 2),
				total.saturating_sub(self.max_visible),
			),
		);
		let end = min(start + self.max_visible, total);

		let cursor_str = self.theme.symbols.cursor;
		let cursor_width = rho_text::width::visible_width_str(cursor_str);

		for i in start..end {
			let item_idx = self.filtered_items[i];
			let item = &self.items[item_idx];
			let is_selected = i == self.selected_index;
			let display_value = if item.label.is_empty() {
				&item.value
			} else {
				&item.label
			};

			let line = if is_selected {
				let prefix = format!("{cursor_str} ");
				let prefix_width = cursor_width + 1;

				if item.description.is_some() && w > 40 {
					let max_value_width = min(30, w.saturating_sub(prefix_width + 4));
					let truncated_value = Self::truncate_no_ellipsis(display_value, max_value_width);
					let trunc_len = truncated_value.len();
					let spacing = make_padding(max(1, 32usize.saturating_sub(trunc_len)));
					let desc_start = prefix_width + trunc_len + spacing.len();
					let remaining = w.saturating_sub(desc_start + 2);

					if remaining > 10 {
						let truncated_desc = Self::truncate_no_ellipsis(
							item.description.as_deref().unwrap_or(""),
							remaining,
						);
						(self.theme.selected_text)(&format!(
							"{prefix}{truncated_value}{spacing}{truncated_desc}"
						))
					} else {
						let max_w = w.saturating_sub(prefix_width + 2);
						(self.theme.selected_text)(&format!(
							"{prefix}{}",
							Self::truncate_no_ellipsis(display_value, max_w)
						))
					}
				} else {
					let max_w = w.saturating_sub(prefix_width + 2);
					(self.theme.selected_text)(&format!(
						"{prefix}{}",
						Self::truncate_no_ellipsis(display_value, max_w)
					))
				}
			} else {
				let prefix = make_padding(cursor_width + 1);

				if item.description.is_some() && w > 40 {
					let max_value_width = min(30, w.saturating_sub(prefix.len() + 4));
					let truncated_value = Self::truncate_no_ellipsis(display_value, max_value_width);
					let trunc_len = truncated_value.len();
					let spacing = make_padding(max(1, 32usize.saturating_sub(trunc_len)));
					let desc_start = prefix.len() + trunc_len + spacing.len();
					let remaining = w.saturating_sub(desc_start + 2);

					if remaining > 10 {
						let truncated_desc = Self::truncate_no_ellipsis(
							item.description.as_deref().unwrap_or(""),
							remaining,
						);
						let desc_text = (self.theme.description)(&format!("{spacing}{truncated_desc}"));
						format!("{prefix}{truncated_value}{desc_text}")
					} else {
						let max_w = w.saturating_sub(prefix.len() + 2);
						format!("{prefix}{}", Self::truncate_no_ellipsis(display_value, max_w))
					}
				} else {
					let max_w = w.saturating_sub(prefix.len() + 2);
					format!("{prefix}{}", Self::truncate_no_ellipsis(display_value, max_w))
				}
			};

			lines.push(line);
		}

		// Scroll indicator
		if start > 0 || end < total {
			let scroll_text = format!("  ({}/{})", self.selected_index + 1, total);
			lines.push((self.theme.scroll_info)(&Self::truncate_no_ellipsis(
				&scroll_text,
				w.saturating_sub(2),
			)));
		}

		lines
	}

	fn handle_input(&mut self, data: &str) -> InputResult {
		let bytes = data.as_bytes();
		let total = self.filtered_items.len();
		if total == 0 {
			return InputResult::Ignored;
		}

		if crate::keys::match_key::matches_key(bytes, "up", false) {
			self.selected_index = if self.selected_index == 0 {
				total - 1
			} else {
				self.selected_index - 1
			};
			self.notify_selection_change();
			return InputResult::Consumed;
		}

		if crate::keys::match_key::matches_key(bytes, "down", false) {
			self.selected_index = if self.selected_index >= total - 1 {
				0
			} else {
				self.selected_index + 1
			};
			self.notify_selection_change();
			return InputResult::Consumed;
		}

		if crate::keys::match_key::matches_key(bytes, "enter", false)
			|| crate::keys::match_key::matches_key(bytes, "return", false)
			|| data == "\n"
		{
			if let Some(ref mut cb) = self.on_select
				&& let Some(&idx) = self.filtered_items.get(self.selected_index)
			{
				cb(&self.items[idx]);
			}
			return InputResult::Consumed;
		}

		if crate::keys::match_key::matches_key(bytes, "escape", false)
			|| crate::keys::match_key::matches_key(bytes, "esc", false)
			|| crate::keys::match_key::matches_key(bytes, "ctrl+c", false)
		{
			if let Some(ref mut cb) = self.on_cancel {
				cb();
			}
			return InputResult::Consumed;
		}

		InputResult::Ignored
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plain_theme() -> SelectListTheme {
		SelectListTheme {
			selected_prefix: Box::new(|s| s.to_owned()),
			selected_text:   Box::new(|s| s.to_owned()),
			description:     Box::new(|s| s.to_owned()),
			scroll_info:     Box::new(|s| s.to_owned()),
			no_match:        Box::new(|s| s.to_owned()),
			symbols:         crate::symbols::SymbolTheme {
				cursor:         ">",
				input_cursor:   "|",
				box_round:      crate::symbols::RoundedBoxSymbols {
					top_left:     "╭",
					top_right:    "╮",
					bottom_left:  "╰",
					bottom_right: "╯",
					horizontal:   "─",
					vertical:     "│",
				},
				box_sharp:      crate::symbols::BoxSymbols {
					top_left:     "┌",
					top_right:    "┐",
					bottom_left:  "└",
					bottom_right: "┘",
					horizontal:   "─",
					vertical:     "│",
					tee_down:     "┬",
					tee_up:       "┴",
					tee_left:     "┤",
					tee_right:    "├",
					cross:        "┼",
				},
				table:          crate::symbols::BoxSymbols {
					top_left:     "┌",
					top_right:    "┐",
					bottom_left:  "└",
					bottom_right: "┘",
					horizontal:   "─",
					vertical:     "│",
					tee_down:     "┬",
					tee_up:       "┴",
					tee_left:     "┤",
					tee_right:    "├",
					cross:        "┼",
				},
				tree:           crate::symbols::TreeSymbols {
					branch:   "├─",
					last:     "╰─",
					vertical: "│",
				},
				quote_border:   "│",
				hr_char:        "─",
				spinner_frames: &["⠋"],
			},
		}
	}

	fn make_items() -> Vec<SelectItem> {
		vec![
			SelectItem::new("alpha", "Alpha"),
			SelectItem::new("beta", "Beta").with_description("Second letter"),
			SelectItem::new("gamma", "Gamma"),
		]
	}

	#[test]
	fn test_select_list_render() {
		let mut list = SelectList::new(make_items(), 10, plain_theme());
		let lines = list.render(60);
		assert_eq!(lines.len(), 3);
		assert!(lines[0].contains("Alpha"));
		assert!(lines[1].contains("Beta"));
	}

	#[test]
	fn test_select_list_nav() {
		let mut list = SelectList::new(make_items(), 10, plain_theme());
		assert_eq!(list.selected_index, 0);
		list.handle_input("\x1b[B"); // down
		assert_eq!(list.selected_index, 1);
		list.handle_input("\x1b[B"); // down
		assert_eq!(list.selected_index, 2);
		// Wrap
		list.handle_input("\x1b[B"); // down
		assert_eq!(list.selected_index, 0);
		// Up wraps to end
		list.handle_input("\x1b[A"); // up
		assert_eq!(list.selected_index, 2);
	}

	#[test]
	fn test_select_list_filter() {
		let mut list = SelectList::new(make_items(), 10, plain_theme());
		list.set_filter("al");
		assert_eq!(list.filtered_items.len(), 1);
		let lines = list.render(60);
		assert!(lines[0].contains("Alpha"));
	}

	#[test]
	fn test_select_list_empty_filter() {
		let mut list = SelectList::new(make_items(), 10, plain_theme());
		list.set_filter("xyz");
		assert!(list.filtered_items.is_empty());
		let lines = list.render(60);
		assert!(lines[0].contains("No matching"));
	}

	#[test]
	fn test_select_list_scroll_indicator() {
		let mut list = SelectList::new(make_items(), 2, plain_theme());
		let lines = list.render(60);
		// 2 visible + scroll indicator
		assert_eq!(lines.len(), 3);
		assert!(lines[2].contains("1/3"));
	}
}
