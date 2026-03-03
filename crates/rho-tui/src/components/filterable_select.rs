//! Filterable select component — tabbed, searchable select list.
//!
//! Composes `TabBar` + `Input` + `SelectList` into a reusable filterable
//! selector. Supports fuzzy search across items and tab-based category
//! filtering.

use super::{
	input::Input,
	select_list::{SelectItem, SelectList, SelectListTheme},
	tab_bar::{Tab, TabBar, TabBarTheme},
};
use crate::{
	component::{Component, InputResult},
	fuzzy::fuzzy_filter,
};

/// Item for `FilterableSelect`. Extends `SelectItem` with a `tab_id` for
/// tab-based filtering.
#[derive(Debug, Clone)]
pub struct FilterableSelectItem {
	pub value:       String,
	pub label:       String,
	pub description: Option<String>,
	pub tab_id:      String,
}

/// Combined theme for the `FilterableSelect` component.
pub struct FilterableSelectTheme {
	pub tab_bar:     TabBarTheme,
	pub select_list: SelectListTheme,
	pub search_hint: Box<dyn Fn(&str) -> String>,
	pub border:      Box<dyn Fn(&str) -> String>,
}

/// A tabbed, searchable select list.
///
/// Composes `TabBar`, `Input`, and `SelectList` vertically. Filters items
/// by active tab and fuzzy search query simultaneously.
pub struct FilterableSelect {
	items:       Vec<FilterableSelectItem>,
	tab_bar:     TabBar,
	search:      Input,
	list:        SelectList,
	theme:       FilterableSelectTheme,
	all_tab_id:  String,
	max_visible: usize,
	cancelled:   bool,
}

impl FilterableSelect {
	/// Create a new filterable select.
	///
	/// The first tab must be the "All" tab (shows all items regardless of
	/// `tab_id`). Items are converted to `SelectItem`s for the inner
	/// `SelectList`.
	pub fn new(
		tabs: Vec<Tab>,
		items: Vec<FilterableSelectItem>,
		max_visible: usize,
		theme: FilterableSelectTheme,
	) -> Self {
		let all_tab_id = tabs
			.first()
			.map_or_else(|| "all".to_owned(), |t| t.id.clone());
		let tab_bar = TabBar::new("Provider", tabs, TabBarTheme::plain());
		let search = Input::new();

		// Build initial SelectList from all items.
		let select_items = items_to_select_items(&items);
		let list = SelectList::new(select_items, max_visible, SelectListTheme::plain());

		let mut s =
			Self { items, tab_bar, search, list, theme, all_tab_id, max_visible, cancelled: false };
		// Rebuild with the actual theme.
		s.rebuild_list(max_visible);
		s
	}

	/// Value of the currently highlighted item.
	pub fn selected_value(&self) -> Option<&str> {
		self.list.selected_item().map(|item| item.value.as_str())
	}

	/// Whether Esc/Ctrl+C was pressed.
	pub const fn is_cancelled(&self) -> bool {
		self.cancelled
	}

	/// Apply tab + search filter and rebuild the inner `SelectList`.
	fn apply_filter(&mut self) {
		let query = self.search.value().to_owned();
		let active_tab_id = self.tab_bar.active_tab().map(|t| t.id.clone());
		let is_all = active_tab_id.as_deref() == Some(self.all_tab_id.as_str());

		// Step 1: filter by tab.
		let tab_filtered: Vec<&FilterableSelectItem> = if is_all {
			self.items.iter().collect()
		} else {
			self
				.items
				.iter()
				.filter(|item| active_tab_id.as_deref() == Some(item.tab_id.as_str()))
				.collect()
		};

		// Step 2: fuzzy filter by search query.
		let matching: Vec<&FilterableSelectItem> = if query.is_empty() {
			tab_filtered
		} else {
			// fuzzy_filter expects &[T] — pass the slice of refs and extract the inner ref.
			let fuzzy_results: Vec<&&FilterableSelectItem> =
				fuzzy_filter(&tab_filtered, &query, |item: &&FilterableSelectItem| &item.label);
			fuzzy_results.into_iter().copied().collect()
		};

		// Step 3: rebuild SelectList with filtered items.
		let select_items: Vec<SelectItem> = matching
			.iter()
			.map(|item| item_to_select_item(item))
			.collect();
		self.list = SelectList::new(select_items, self.max_visible, SelectListTheme::plain());
	}

	/// Rebuild the list from scratch (used during construction).
	fn rebuild_list(&mut self, max_visible: usize) {
		let select_items = items_to_select_items(&self.items);
		self.list = SelectList::new(select_items, max_visible, SelectListTheme::plain());
	}
}

impl Component for FilterableSelect {
	fn render(&mut self, width: u16) -> Vec<String> {
		let w = width as usize;
		let mut lines = Vec::new();

		// 1. Tab bar
		lines.extend(self.tab_bar.render(width));

		// 2. Search input
		lines.extend(self.search.render(width));

		// 3. Separator line
		let separator_width = w.saturating_sub(4);
		let separator = "\u{2500}".repeat(separator_width.min(60));
		lines.push(format!("  {}", (self.theme.border)(&separator)));

		// 4. Filtered list
		lines.extend(self.list.render(width));

		lines
	}

	fn handle_input(&mut self, data: &str) -> InputResult {
		let bytes = data.as_bytes();

		// 1. Esc / Ctrl+C → cancel
		if crate::keys::match_key::matches_key(bytes, "escape", false)
			|| crate::keys::match_key::matches_key(bytes, "esc", false)
			|| data == "\x03"
		{
			self.cancelled = true;
			return InputResult::Consumed;
		}

		// 2. Enter → submit selected item
		if crate::keys::match_key::matches_key(bytes, "enter", false)
			|| crate::keys::match_key::matches_key(bytes, "return", false)
			|| data == "\n"
		{
			if let Some(value) = self.selected_value() {
				return InputResult::Submit(value.to_owned());
			}
			return InputResult::Consumed;
		}

		// 3. Up/Down → navigate list
		if crate::keys::match_key::matches_key(bytes, "up", false)
			|| crate::keys::match_key::matches_key(bytes, "down", false)
		{
			self.list.handle_input(data);
			return InputResult::Consumed;
		}

		// 4. Tab/Shift+Tab → cycle tabs, then re-filter
		if crate::keys::match_key::matches_key(bytes, "tab", false)
			|| crate::keys::match_key::matches_key(bytes, "shift+tab", false)
		{
			self.tab_bar.handle_input(data);
			self.apply_filter();
			return InputResult::Consumed;
		}

		// 5. Backspace / character input → delegate to search Input, then re-filter On
		//    first character typed: auto-switch to "All" tab.
		let was_empty = self.search.value().is_empty();

		// Check if this is a backspace
		let is_backspace =
			crate::keys::match_key::matches_key(bytes, "backspace", false) || data == "\x7f";

		if is_backspace {
			self.search.handle_input(data);
			self.apply_filter();
			return InputResult::Consumed;
		}

		// Try forwarding to the search input
		let result = self.search.handle_input(data);
		if matches!(result, InputResult::Consumed) {
			// Auto-switch to "All" tab on first character typed
			if was_empty && !self.search.value().is_empty() {
				self.tab_bar.set_active_index(0);
			}
			self.apply_filter();
			return InputResult::Consumed;
		}

		// FilterableSelect consumes all input to prevent leakage.
		InputResult::Consumed
	}
}

/// Convert a `FilterableSelectItem` to a `SelectItem`.
fn item_to_select_item(item: &FilterableSelectItem) -> SelectItem {
	let mut si = SelectItem::new(&item.value, &item.label);
	if let Some(ref desc) = item.description {
		si = si.with_description(desc);
	}
	si
}

/// Convert a slice of `FilterableSelectItem`s to `SelectItem`s.
fn items_to_select_items(items: &[FilterableSelectItem]) -> Vec<SelectItem> {
	items.iter().map(item_to_select_item).collect()
}

// ── SelectListTheme::plain() ────────────────────────────────────────────

impl SelectListTheme {
	/// Create a plain (unstyled) theme — for use in tests and as fallback.
	pub fn plain() -> Self {
		Self {
			selected_prefix: Box::new(|s| s.to_owned()),
			selected_text:   Box::new(|s| s.to_owned()),
			description:     Box::new(|s| s.to_owned()),
			scroll_info:     Box::new(|s| s.to_owned()),
			no_match:        Box::new(|s| s.to_owned()),
			symbols:         crate::symbols::SymbolTheme::plain(),
		}
	}
}

// ── SymbolTheme::plain() ────────────────────────────────────────────────

impl crate::symbols::SymbolTheme {
	/// Create a plain symbol theme for testing.
	pub const fn plain() -> Self {
		Self {
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
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_tabs() -> Vec<Tab> {
		vec![Tab::new("all", "All"), Tab::new("anthropic", "Anthropic"), Tab::new("openai", "OpenAI")]
	}

	fn make_items() -> Vec<FilterableSelectItem> {
		vec![
			FilterableSelectItem {
				value:       "anthropic/claude-sonnet".to_owned(),
				label:       "Claude Sonnet 4.5  200K [DEFAULT]".to_owned(),
				description: Some("anthropic  images".to_owned()),
				tab_id:      "anthropic".to_owned(),
			},
			FilterableSelectItem {
				value:       "anthropic/claude-haiku".to_owned(),
				label:       "Claude Haiku 4.5  200K [FAST]".to_owned(),
				description: Some("anthropic  images".to_owned()),
				tab_id:      "anthropic".to_owned(),
			},
			FilterableSelectItem {
				value:       "openai/gpt-4o".to_owned(),
				label:       "GPT-4o  128K".to_owned(),
				description: Some("openai  images".to_owned()),
				tab_id:      "openai".to_owned(),
			},
			FilterableSelectItem {
				value:       "openai/gpt-4o-mini".to_owned(),
				label:       "GPT-4o Mini  128K".to_owned(),
				description: Some("openai".to_owned()),
				tab_id:      "openai".to_owned(),
			},
		]
	}

	fn plain_theme() -> FilterableSelectTheme {
		FilterableSelectTheme {
			tab_bar:     TabBarTheme::plain(),
			select_list: SelectListTheme::plain(),
			search_hint: Box::new(|s| s.to_owned()),
			border:      Box::new(|s| s.to_owned()),
		}
	}

	#[test]
	fn test_render_shows_tabs_search_list() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());
		let lines = fs.render(80);
		// Should have: tab bar line(s), search input line, separator, list items
		assert!(lines.len() >= 4, "Expected at least 4 lines, got {}", lines.len());
		// Tab bar should contain tab labels
		let all_text = lines.join("\n");
		assert!(all_text.contains("All"), "Should contain 'All' tab");
		assert!(all_text.contains("Anthropic"), "Should contain 'Anthropic' tab");
		assert!(all_text.contains("OpenAI"), "Should contain 'OpenAI' tab");
		// List should show items
		assert!(all_text.contains("Claude Sonnet"), "Should contain first item");
	}

	#[test]
	fn test_tab_switching_filters_items() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());

		// Initially on "All" tab — all 4 items visible
		let lines = fs.render(80);
		let all_text = lines.join("\n");
		assert!(all_text.contains("Claude Sonnet"));
		assert!(all_text.contains("GPT-4o"));

		// Switch to "Anthropic" tab (Tab key)
		fs.handle_input("\t"); // Tab
		let lines = fs.render(80);
		let all_text = lines.join("\n");
		assert!(all_text.contains("Claude Sonnet"), "Anthropic items should be visible");
		assert!(all_text.contains("Claude Haiku"), "Anthropic items should be visible");
		assert!(!all_text.contains("GPT-4o Mini"), "OpenAI items should be filtered out");

		// Switch to "OpenAI" tab (Tab key again)
		fs.handle_input("\t");
		let lines = fs.render(80);
		let all_text = lines.join("\n");
		assert!(!all_text.contains("Claude"), "Anthropic items should be filtered out");
		assert!(all_text.contains("GPT-4o"), "OpenAI items should be visible");
	}

	#[test]
	fn test_search_filters_fuzzy() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());

		// Type "haiku" to filter
		fs.handle_input("h");
		fs.handle_input("a");
		fs.handle_input("i");
		fs.handle_input("k");
		fs.handle_input("u");

		let lines = fs.render(80);
		let all_text = lines.join("\n");
		assert!(all_text.contains("Haiku"), "Haiku should match fuzzy search");
		// Other items should be filtered out (or at least Haiku should be
		// prioritized)
	}

	#[test]
	fn test_auto_switch_to_all_on_type() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());

		// Switch to "Anthropic" tab
		fs.handle_input("\t");
		assert_eq!(fs.tab_bar.active_index(), 1, "Should be on Anthropic tab");

		// Type a character — should auto-switch to "All" tab
		fs.handle_input("g");
		assert_eq!(fs.tab_bar.active_index(), 0, "Should auto-switch to All tab");
	}

	#[test]
	fn test_enter_returns_submit() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());
		let result = fs.handle_input("\r"); // Enter
		match result {
			InputResult::Submit(value) => {
				assert_eq!(value, "anthropic/claude-sonnet", "Should submit first item's value");
			},
			other => panic!("Expected Submit, got {:?}", other),
		}
	}

	#[test]
	fn test_esc_sets_cancelled() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());
		assert!(!fs.is_cancelled());
		fs.handle_input("\x1b"); // Esc
		assert!(fs.is_cancelled());
	}

	#[test]
	fn test_up_down_navigates() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());

		// Initially first item selected
		assert_eq!(fs.selected_value(), Some("anthropic/claude-sonnet"));

		// Down
		fs.handle_input("\x1b[B");
		assert_eq!(fs.selected_value(), Some("anthropic/claude-haiku"));

		// Down
		fs.handle_input("\x1b[B");
		assert_eq!(fs.selected_value(), Some("openai/gpt-4o"));

		// Up
		fs.handle_input("\x1b[A");
		assert_eq!(fs.selected_value(), Some("anthropic/claude-haiku"));
	}

	#[test]
	fn test_empty_search_shows_all_in_tab() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());

		// Switch to OpenAI tab
		fs.handle_input("\t"); // Anthropic
		fs.handle_input("\t"); // OpenAI

		// Empty search should show all OpenAI items
		let lines = fs.render(80);
		let all_text = lines.join("\n");
		assert!(all_text.contains("GPT-4o"), "Should show GPT-4o");
		assert!(all_text.contains("GPT-4o Mini"), "Should show GPT-4o Mini");
		assert!(!all_text.contains("Claude"), "Should not show Anthropic items");
	}

	#[test]
	fn test_ctrl_c_sets_cancelled() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());
		assert!(!fs.is_cancelled());
		fs.handle_input("\x03"); // Ctrl+C
		assert!(fs.is_cancelled());
	}

	#[test]
	fn test_backspace_clears_search() {
		let mut fs = FilterableSelect::new(make_tabs(), make_items(), 10, plain_theme());

		// Type something
		fs.handle_input("g");
		fs.handle_input("p");
		assert!(!fs.search.value().is_empty());

		// Backspace twice
		fs.handle_input("\x7f");
		fs.handle_input("\x7f");
		assert!(fs.search.value().is_empty());

		// All items should be visible again
		let lines = fs.render(80);
		let all_text = lines.join("\n");
		assert!(all_text.contains("Claude Sonnet"));
		assert!(all_text.contains("GPT-4o"));
	}
}
