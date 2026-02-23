//! Bridge between [`CombinedAutocompleteProvider`] (rho-tui autocomplete
//! module) and the editor's [`AutocompleteProvider`] trait.

use std::path::PathBuf;

use rho_tui::components::{
	autocomplete::{
		AutocompleteItem, AutocompleteProvider as AutocompleteProviderInner,
		CombinedAutocompleteProvider, CommandEntry,
	},
	editor::{AutocompleteProvider, AutocompleteSuggestions, CompletionResult},
	select_list::SelectItem,
};

use crate::commands::COMMANDS;

/// Autocomplete provider for the rho editor.
///
/// Wraps [`CombinedAutocompleteProvider`] and converts between the autocomplete
/// module's types and the editor's expected types.
pub struct RhoAutocompleteProvider {
	inner: CombinedAutocompleteProvider,
}

impl RhoAutocompleteProvider {
	/// Create a new provider with slash commands from the rho registry.
	pub fn new(base_path: PathBuf) -> Self {
		let commands = build_command_entries();
		Self { inner: CombinedAutocompleteProvider::new(commands, base_path) }
	}
}

impl AutocompleteProvider for RhoAutocompleteProvider {
	fn get_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<AutocompleteSuggestions> {
		let result = self.inner.get_suggestions(lines, cursor_line, cursor_col)?;
		Some(AutocompleteSuggestions {
			items:  result.items.into_iter().map(to_select_item).collect(),
			prefix: result.prefix,
		})
	}

	fn apply_completion(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
		item: &SelectItem,
		prefix: &str,
	) -> CompletionResult {
		let ac_item = to_autocomplete_item(item);
		let result = self.inner.apply_completion(lines, cursor_line, cursor_col, &ac_item, prefix);
		CompletionResult {
			lines:       result.lines,
			cursor_line: result.cursor_line,
			cursor_col:  result.cursor_col,
		}
	}

	fn get_inline_hint(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<String> {
		self.inner.get_inline_hint(lines, cursor_line, cursor_col)
	}

	fn should_trigger_file_completion(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> bool {
		CombinedAutocompleteProvider::should_trigger_file_completion(lines, cursor_line, cursor_col)
	}

	fn get_force_file_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<AutocompleteSuggestions> {
		let result = self.inner.get_force_file_suggestions(lines, cursor_line, cursor_col)?;
		Some(AutocompleteSuggestions {
			items:  result.items.into_iter().map(to_select_item).collect(),
			prefix: result.prefix,
		})
	}
}

// ── Type conversions ────────────────────────────────────────────────────

fn to_select_item(item: AutocompleteItem) -> SelectItem {
	SelectItem {
		value:       item.value,
		label:       item.label,
		description: item.description,
		hint:        item.hint,
	}
}

fn to_autocomplete_item(item: &SelectItem) -> AutocompleteItem {
	AutocompleteItem {
		value:       item.value.clone(),
		label:       item.label.clone(),
		description: item.description.clone(),
		hint:        item.hint.clone(),
	}
}

// ── Command registry conversion ─────────────────────────────────────────

/// Build [`CommandEntry`] list from omp's static [`COMMANDS`] registry.
fn build_command_entries() -> Vec<CommandEntry> {
	COMMANDS
		.iter()
		.map(|cmd| {
			CommandEntry::Item(
				AutocompleteItem::new(cmd.name, cmd.name).with_description(cmd.description),
			)
		})
		.collect()
}
