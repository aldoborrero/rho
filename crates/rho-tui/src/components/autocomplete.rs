//! Autocomplete system — slash command completion, file path completion, and
//! fuzzy file search.
//!
//! Provides an `AutocompleteProvider` trait and a
//! `CombinedAutocompleteProvider` that handles slash commands, file path prefix
//! completion via `readdir`, and fuzzy file discovery via
//! `ignore::WalkBuilder`.

use std::{
	cmp::Reverse,
	collections::HashMap,
	path::{Path, PathBuf},
	time::{Duration, Instant},
};

/// Callback type for argument completions.
pub type ArgCompletionsFn = Box<dyn Fn(&str) -> Option<Vec<AutocompleteItem>>>;

/// Callback type for inline hint.
pub type InlineHintFn = Box<dyn Fn(&str) -> Option<String>>;

// ── Types ────────────────────────────────────────────────────────────────

/// A single autocomplete suggestion.
#[derive(Debug, Clone)]
pub struct AutocompleteItem {
	pub value:       String,
	pub label:       String,
	pub description: Option<String>,
	/// Dim hint text shown inline when this item is selected.
	pub hint:        Option<String>,
}

impl AutocompleteItem {
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

/// A slash command that can provide argument completions and inline hints.
pub struct SlashCommand {
	pub name:                     String,
	pub description:              Option<String>,
	/// Function to get argument completions for this command.
	pub get_argument_completions: Option<ArgCompletionsFn>,
	/// Return inline hint text for the current argument state.
	pub get_inline_hint:          Option<InlineHintFn>,
}

/// A registered command — either a full slash command or a simple autocomplete
/// item.
pub enum CommandEntry {
	Slash(SlashCommand),
	Item(AutocompleteItem),
}

impl CommandEntry {
	fn name(&self) -> &str {
		match self {
			Self::Slash(cmd) => &cmd.name,
			Self::Item(item) => &item.value,
		}
	}

	fn label(&self) -> &str {
		match self {
			Self::Slash(cmd) => &cmd.name,
			Self::Item(item) => &item.label,
		}
	}

	fn description(&self) -> Option<&str> {
		match self {
			Self::Slash(cmd) => cmd.description.as_deref(),
			Self::Item(item) => item.description.as_deref(),
		}
	}
}

/// Result of `get_suggestions`.
pub struct SuggestionResult {
	pub items:  Vec<AutocompleteItem>,
	/// What we're matching against (e.g., "/" or "src/").
	pub prefix: String,
}

/// Result of `apply_completion`.
pub struct CompletionResult {
	pub lines:       Vec<String>,
	pub cursor_line: usize,
	pub cursor_col:  usize,
}

/// Autocomplete provider trait.
pub trait AutocompleteProvider {
	/// Get autocomplete suggestions for current text/cursor position.
	fn get_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<SuggestionResult>;

	/// Apply the selected item and return new text + cursor position.
	fn apply_completion(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
		item: &AutocompleteItem,
		prefix: &str,
	) -> CompletionResult;

	/// Get inline hint text to show as dim ghost text after the cursor.
	fn get_inline_hint(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<String>;
}

// ── Pure helpers ─────────────────────────────────────────────────────────

const PATH_DELIMITERS: &[char] = &[' ', '\t', '"', '\'', '='];

fn is_path_delimiter(c: char) -> bool {
	PATH_DELIMITERS.contains(&c)
}

fn find_last_delimiter(text: &str) -> Option<usize> {
	text
		.char_indices()
		.rev()
		.find(|&(_, c)| is_path_delimiter(c))
		.map(|(i, _)| i)
}

fn find_unclosed_quote_start(text: &str) -> Option<usize> {
	let mut in_quotes = false;
	let mut quote_start = 0;

	for (i, c) in text.char_indices() {
		if c == '"' {
			in_quotes = !in_quotes;
			if in_quotes {
				quote_start = i;
			}
		}
	}

	if in_quotes { Some(quote_start) } else { None }
}

fn is_token_start(text: &str, byte_index: usize) -> bool {
	if byte_index == 0 {
		return true;
	}
	text[..byte_index]
		.chars()
		.next_back()
		.is_some_and(is_path_delimiter)
}

fn extract_quoted_prefix(text: &str) -> Option<&str> {
	let quote_start = find_unclosed_quote_start(text)?;

	// Check for @" prefix
	if quote_start > 0 && text.as_bytes().get(quote_start - 1) == Some(&b'@') {
		if !is_token_start(text, quote_start - 1) {
			return None;
		}
		return Some(&text[quote_start - 1..]);
	}

	if !is_token_start(text, quote_start) {
		return None;
	}

	Some(&text[quote_start..])
}

struct ParsedPrefix<'a> {
	raw:       &'a str,
	is_at:     bool,
	is_quoted: bool,
}

fn parse_path_prefix(prefix: &str) -> ParsedPrefix<'_> {
	if let Some(rest) = prefix.strip_prefix("@\"") {
		return ParsedPrefix { raw: rest, is_at: true, is_quoted: true };
	}
	if let Some(rest) = prefix.strip_prefix('"') {
		return ParsedPrefix { raw: rest, is_at: false, is_quoted: true };
	}
	if let Some(rest) = prefix.strip_prefix('@') {
		return ParsedPrefix { raw: rest, is_at: true, is_quoted: false };
	}
	ParsedPrefix { raw: prefix, is_at: false, is_quoted: false }
}

fn build_completion_value(path: &str, is_directory: bool, is_at: bool, is_quoted: bool) -> String {
	let needs_quotes = is_quoted || path.contains(' ');
	let prefix = if is_at { "@" } else { "" };

	if !needs_quotes {
		return format!("{prefix}{path}");
	}

	let open_quote = format!("{prefix}\"");
	let close_quote = if is_directory { "" } else { "\"" };
	format!("{open_quote}{path}{close_quote}")
}

/// Check if query is a subsequence of target (fuzzy match).
fn fuzzy_match_subsequence(query: &str, target: &str) -> bool {
	if query.is_empty() {
		return true;
	}
	if query.len() > target.len() {
		return false;
	}

	let mut qi = query.chars();
	let mut current = qi.next();

	for tc in target.chars() {
		if let Some(qc) = current {
			if qc == tc {
				current = qi.next();
			}
		} else {
			break;
		}
	}

	current.is_none()
}

/// Score a fuzzy match. Higher = better match.
/// Prioritizes: exact match > starts-with > contains > subsequence.
fn fuzzy_score(query: &str, target: &str) -> u32 {
	if query.is_empty() {
		return 1;
	}
	if target == query {
		return 100;
	}
	if target.starts_with(query) {
		return 80;
	}
	if target.contains(query) {
		return 60;
	}

	// Subsequence match — score by tightness
	let mut qi = query.chars().peekable();
	let mut gaps: u32 = 0;
	let mut last_match_idx: Option<usize> = None;

	for (ti, tc) in target.char_indices() {
		if qi.peek().is_some_and(|&qc| qc == tc) {
			if let Some(last) = last_match_idx
				&& ti - last > 1
			{
				gaps += 1;
			}
			last_match_idx = Some(ti);
			qi.next();
		}
	}

	if qi.peek().is_some() {
		return 0;
	}

	// Base score 40 for subsequence, minus penalty for gaps
	40u32.saturating_sub(gaps * 5).max(1)
}

// ── Directory cache ──────────────────────────────────────────────────────

struct DirCacheEntry {
	entries:   Vec<DirEntry>,
	timestamp: Instant,
}

#[derive(Debug, Clone)]
struct DirEntry {
	name:       String,
	is_dir:     bool,
	is_symlink: bool,
}

const DIR_CACHE_TTL: Duration = Duration::from_secs(2);
const DIR_CACHE_MAX: usize = 100;

// ── CombinedAutocompleteProvider ─────────────────────────────────────────

/// Combined provider that handles slash commands, file paths, and fuzzy file
/// search.
pub struct CombinedAutocompleteProvider {
	commands:  Vec<CommandEntry>,
	base_path: PathBuf,
	dir_cache: HashMap<PathBuf, DirCacheEntry>,
}

impl CombinedAutocompleteProvider {
	pub fn new(commands: Vec<CommandEntry>, base_path: PathBuf) -> Self {
		Self { commands, base_path, dir_cache: HashMap::new() }
	}

	/// Force file completion (called on Tab key).
	pub fn get_force_file_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<SuggestionResult> {
		let current_line = lines.get(cursor_line).map_or("", String::as_str);
		let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

		// Don't trigger if we're typing a slash command at start of line
		let trimmed = text_before_cursor.trim();
		if trimmed.starts_with('/') && !trimmed.contains(' ') {
			return None;
		}

		let path_match = Self::extract_path_prefix(text_before_cursor, true)?;
		let suggestions = self.get_file_suggestions(&path_match);
		if suggestions.is_empty() {
			return None;
		}

		Some(SuggestionResult { items: suggestions, prefix: path_match })
	}

	/// Check if we should trigger file completion.
	pub fn should_trigger_file_completion(
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> bool {
		let current_line = lines.get(cursor_line).map_or("", String::as_str);
		let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

		let trimmed = text_before_cursor.trim();
		!trimmed.starts_with('/') || trimmed.contains(' ')
	}

	/// Invalidate directory cache.
	pub fn invalidate_dir_cache(&mut self, dir: Option<&Path>) {
		if let Some(d) = dir {
			self.dir_cache.remove(d);
		} else {
			self.dir_cache.clear();
		}
	}

	// ── @ prefix extraction ──────────────────────────────────────────

	fn extract_at_prefix(text: &str) -> Option<String> {
		if let Some(quoted) = extract_quoted_prefix(text)
			&& quoted.starts_with("@\"")
		{
			return Some(quoted.to_owned());
		}

		let last_delim = find_last_delimiter(text);
		let token_start = match last_delim {
			Some(i) => i + 1,
			None => 0,
		};

		if text.as_bytes().get(token_start) == Some(&b'@') {
			return Some(text[token_start..].to_owned());
		}

		None
	}

	// ── Path prefix extraction ───────────────────────────────────────

	fn extract_path_prefix(text: &str, force_extract: bool) -> Option<String> {
		if let Some(quoted) = extract_quoted_prefix(text) {
			return Some(quoted.to_owned());
		}

		let last_delim = find_last_delimiter(text);
		let path_prefix = match last_delim {
			Some(i) => &text[i + 1..],
			None => text,
		};

		if force_extract {
			return Some(path_prefix.to_owned());
		}

		// For natural triggers, return if it looks like a path
		if path_prefix.contains('/') || path_prefix.starts_with('.') || path_prefix.starts_with("~/")
		{
			return Some(path_prefix.to_owned());
		}

		// Empty prefix only after a space
		if path_prefix.is_empty() && text.ends_with(' ') {
			return Some(path_prefix.to_owned());
		}

		None
	}

	// ── Home directory expansion ─────────────────────────────────────

	fn expand_home_path(file_path: &str) -> PathBuf {
		if file_path == "~" {
			return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
		}
		if let Some(rest) = file_path.strip_prefix("~/")
			&& let Some(home) = dirs::home_dir()
		{
			let expanded = home.join(rest);
			// Preserve trailing slash
			if file_path.ends_with('/') && !expanded.to_string_lossy().ends_with('/') {
				return PathBuf::from(format!("{}/", expanded.display()));
			}
			return expanded;
		}
		PathBuf::from(file_path)
	}

	// ── Scoped fuzzy query resolution ────────────────────────────────

	fn resolve_scoped_fuzzy_query(&self, raw_query: &str) -> Option<(PathBuf, String, String)> {
		let slash_index = raw_query.rfind('/')?;

		let display_base = &raw_query[..=slash_index];
		let query = &raw_query[slash_index + 1..];

		let base_dir = if display_base.starts_with("~/") {
			Self::expand_home_path(display_base)
		} else if display_base.starts_with('/') {
			PathBuf::from(display_base)
		} else {
			self.base_path.join(display_base)
		};

		// Verify it's a directory
		if !base_dir.is_dir() {
			return None;
		}

		Some((base_dir, query.to_owned(), display_base.to_owned()))
	}

	fn scoped_path_for_display(display_base: &str, relative_path: &str) -> String {
		if display_base == "/" {
			format!("/{relative_path}")
		} else {
			format!("{display_base}{relative_path}")
		}
	}

	// ── Cached directory listing ─────────────────────────────────────

	fn get_cached_dir_entries(&mut self, search_dir: &Path) -> Vec<DirEntry> {
		let now = Instant::now();

		// Check cache
		if let Some(cached) = self.dir_cache.get(search_dir)
			&& now.duration_since(cached.timestamp) < DIR_CACHE_TTL
		{
			return cached.entries.clone();
		}

		// Read directory
		let entries = match std::fs::read_dir(search_dir) {
			Ok(read_dir) => read_dir
				.filter_map(|entry| {
					let entry = entry.ok()?;
					let name = entry.file_name().to_string_lossy().into_owned();
					let file_type = entry.file_type().ok()?;
					Some(DirEntry {
						name,
						is_dir: file_type.is_dir(),
						is_symlink: file_type.is_symlink(),
					})
				})
				.collect(),
			Err(_) => Vec::new(),
		};

		self
			.dir_cache
			.insert(search_dir.to_path_buf(), DirCacheEntry {
				entries:   entries.clone(),
				timestamp: now,
			});

		// Evict old entries if cache is too large
		if self.dir_cache.len() > DIR_CACHE_MAX {
			let mut sorted: Vec<_> = self
				.dir_cache
				.iter()
				.map(|(k, v)| (k.clone(), v.timestamp))
				.collect();
			sorted.sort_by_key(|(_, ts)| *ts);
			for (key, _) in sorted.iter().take(DIR_CACHE_MAX / 2) {
				self.dir_cache.remove(key);
			}
		}

		entries
	}

	// ── File suggestions (prefix-based directory listing) ────────────

	fn get_file_suggestions(&mut self, prefix: &str) -> Vec<AutocompleteItem> {
		let parsed = parse_path_prefix(prefix);
		let mut expanded_prefix = parsed.raw.to_owned();

		// Handle home directory expansion
		if expanded_prefix.starts_with('~') {
			expanded_prefix = Self::expand_home_path(&expanded_prefix)
				.to_string_lossy()
				.into_owned();
		}

		let is_root_prefix = matches!(parsed.raw, "" | "./" | "../" | "~" | "~/" | "/")
			|| (parsed.is_at && parsed.raw.is_empty());

		let (search_dir, search_prefix) = if is_root_prefix || parsed.raw.ends_with('/') {
			let dir = if parsed.raw.starts_with('~') || expanded_prefix.starts_with('/') {
				PathBuf::from(&expanded_prefix)
			} else {
				self.base_path.join(&expanded_prefix)
			};
			(dir, String::new())
		} else {
			let expanded_path = Path::new(&expanded_prefix);
			let dir_part = expanded_path.parent().unwrap_or_else(|| Path::new("."));
			let file_part = expanded_path
				.file_name()
				.map_or(String::new(), |f| f.to_string_lossy().into_owned());

			let dir = if parsed.raw.starts_with('~') || expanded_prefix.starts_with('/') {
				dir_part.to_path_buf()
			} else {
				self.base_path.join(dir_part)
			};
			(dir, file_part)
		};

		let entries = self.get_cached_dir_entries(&search_dir);
		let search_prefix_lower = search_prefix.to_lowercase();
		let mut suggestions = Vec::new();

		for entry in &entries {
			if !entry.name.to_lowercase().starts_with(&search_prefix_lower) {
				continue;
			}
			// Skip .git directory
			if entry.name == ".git" {
				continue;
			}

			// Check if entry is a directory (resolving symlinks)
			let mut is_directory = entry.is_dir;
			if !is_directory && entry.is_symlink {
				let full_path = search_dir.join(&entry.name);
				is_directory = full_path.is_dir();
			}

			let relative_path = Self::build_relative_path(parsed.raw, &entry.name, is_directory);
			let path_value = if is_directory {
				format!("{relative_path}/")
			} else {
				relative_path
			};

			let value =
				build_completion_value(&path_value, is_directory, parsed.is_at, parsed.is_quoted);

			let label = if is_directory {
				format!("{}/", entry.name)
			} else {
				entry.name.clone()
			};

			suggestions.push(AutocompleteItem { value, label, description: None, hint: None });
		}

		// Sort: directories first, then alphabetically
		suggestions.sort_by(|a, b| {
			let a_is_dir = a.value.ends_with('/') || a.value.ends_with("/\"");
			let b_is_dir = b.value.ends_with('/') || b.value.ends_with("/\"");
			match (a_is_dir, b_is_dir) {
				(true, false) => std::cmp::Ordering::Less,
				(false, true) => std::cmp::Ordering::Greater,
				_ => a.label.cmp(&b.label),
			}
		});

		suggestions
	}

	fn build_relative_path(display_prefix: &str, name: &str, _is_directory: bool) -> String {
		if display_prefix.ends_with('/') {
			format!("{display_prefix}{name}")
		} else if display_prefix.contains('/') {
			if let Some(rest) = display_prefix.strip_prefix("~/") {
				let dir = Path::new(rest)
					.parent()
					.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
				if dir == "." || dir.is_empty() {
					format!("~/{name}")
				} else {
					format!("~/{dir}/{name}")
				}
			} else if display_prefix.starts_with('/') {
				let dir = Path::new(display_prefix)
					.parent()
					.map_or_else(|| "/".to_owned(), |p| p.to_string_lossy().into_owned());
				if dir == "/" {
					format!("/{name}")
				} else {
					format!("{dir}/{name}")
				}
			} else {
				let dir = Path::new(display_prefix)
					.parent()
					.map_or_else(String::new, |p| p.to_string_lossy().into_owned());
				if dir == "." || dir.is_empty() {
					name.to_owned()
				} else {
					format!("{dir}/{name}")
				}
			}
		} else if display_prefix.starts_with('~') {
			format!("~/{name}")
		} else {
			name.to_owned()
		}
	}

	// ── Fuzzy file suggestions (full tree search) ────────────────────

	fn get_fuzzy_file_suggestions(&self, query: &str, is_quoted: bool) -> Vec<AutocompleteItem> {
		let scoped = self.resolve_scoped_fuzzy_query(query);
		let (search_path, fuzzy_query, display_base) = match &scoped {
			Some((base_dir, q, db)) => (base_dir.clone(), q.as_str(), Some(db.as_str())),
			None => (self.base_path.clone(), query, None),
		};

		let matches = Self::fuzzy_find_files(&search_path, fuzzy_query, 100);

		// Filter out .git entries
		let filtered: Vec<_> = matches
			.into_iter()
			.filter(|entry| {
				let normalized = entry.path.replace('\\', "/");
				!normalized.contains("/.git/") && !normalized.ends_with("/.git") && normalized != ".git"
			})
			.take(20)
			.collect();

		let mut suggestions = Vec::new();
		for entry in &filtered {
			let path_without_slash = if entry.is_directory {
				entry.path.strip_suffix('/').unwrap_or(&entry.path)
			} else {
				&entry.path
			};

			let display_path = match display_base {
				Some(db) => Self::scoped_path_for_display(db, path_without_slash),
				None => path_without_slash.to_owned(),
			};

			let entry_name = Path::new(path_without_slash)
				.file_name()
				.map_or_else(|| path_without_slash.to_owned(), |f| f.to_string_lossy().into_owned());

			let completion_path = if entry.is_directory {
				format!("{display_path}/")
			} else {
				display_path.clone()
			};

			let value = build_completion_value(&completion_path, entry.is_directory, true, is_quoted);

			let label = if entry.is_directory {
				format!("{entry_name}/")
			} else {
				entry_name
			};

			suggestions.push(AutocompleteItem {
				value,
				label,
				description: Some(display_path),
				hint: None,
			});
		}

		suggestions
	}

	/// Walk directory tree and fuzzy match against query.
	fn fuzzy_find_files(search_path: &Path, query: &str, max_results: usize) -> Vec<FuzzyFileMatch> {
		let mut walker = ignore::WalkBuilder::new(search_path);
		walker
			.hidden(false) // include hidden files
			.git_ignore(true)
			.git_exclude(true)
			.git_global(true)
			.follow_links(false)
			.sort_by_file_path(|a, b| a.cmp(b));

		let mut results: Vec<FuzzyFileMatch> = Vec::new();

		for entry in walker.build().flatten() {
			let path = entry.path();
			// Skip the root directory itself
			if path == search_path {
				continue;
			}

			let relative = match path.strip_prefix(search_path) {
				Ok(r) => r.to_string_lossy().into_owned(),
				Err(_) => continue,
			};

			if relative.is_empty() {
				continue;
			}

			// Skip .git
			if relative == ".git"
				|| relative.starts_with(".git/")
				|| relative.contains("/.git/")
				|| relative.contains("/.git")
			{
				continue;
			}

			let is_directory = path.is_dir();

			if query.is_empty() {
				results.push(FuzzyFileMatch {
					path: if is_directory {
						format!("{relative}/")
					} else {
						relative
					},
					is_directory,
					score: 0.0,
				});
				if results.len() >= max_results {
					break;
				}
				continue;
			}

			let m = crate::fuzzy::fuzzy_match(query, &relative);
			if m.matches {
				results.push(FuzzyFileMatch {
					path: if is_directory {
						format!("{relative}/")
					} else {
						relative
					},
					is_directory,
					score: m.score,
				});
			}
		}

		// Sort by score (lower = better for fuzzy_match)
		results.sort_by(|a, b| {
			a.score
				.partial_cmp(&b.score)
				.unwrap_or(std::cmp::Ordering::Equal)
		});
		results.truncate(max_results);
		results
	}
}

#[derive(Debug)]
struct FuzzyFileMatch {
	path:         String,
	is_directory: bool,
	score:        f64,
}

// ── AutocompleteProvider impl ────────────────────────────────────────────

impl AutocompleteProvider for CombinedAutocompleteProvider {
	fn get_suggestions(
		&mut self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<SuggestionResult> {
		let current_line = lines.get(cursor_line).map_or("", String::as_str);
		let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

		// Check for @ file reference (fuzzy search)
		if let Some(at_prefix) = Self::extract_at_prefix(text_before_cursor) {
			let parsed = parse_path_prefix(&at_prefix);
			let suggestions = if parsed.raw.is_empty() {
				self.get_file_suggestions("@")
			} else {
				let fuzzy = self.get_fuzzy_file_suggestions(parsed.raw, parsed.is_quoted);
				if fuzzy.is_empty() {
					// Fallback to prefix-based suggestions
					let fallback = self.get_file_suggestions(&at_prefix);
					if fallback.is_empty() {
						return None;
					}
					return Some(SuggestionResult { items: fallback, prefix: at_prefix });
				}
				fuzzy
			};

			if suggestions.is_empty() {
				return None;
			}

			return Some(SuggestionResult { items: suggestions, prefix: at_prefix });
		}

		// Check for slash commands
		if text_before_cursor.starts_with('/') {
			return self.get_slash_command_suggestions(text_before_cursor);
		}

		// Check for file paths
		let path_match = Self::extract_path_prefix(text_before_cursor, false)?;
		let suggestions = self.get_file_suggestions(&path_match);
		if suggestions.is_empty() {
			return None;
		}

		Some(SuggestionResult { items: suggestions, prefix: path_match })
	}

	fn apply_completion(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
		item: &AutocompleteItem,
		prefix: &str,
	) -> CompletionResult {
		let current_line = lines.get(cursor_line).map_or("", String::as_str);
		let prefix_start = cursor_col.saturating_sub(prefix.len());
		let before_prefix = &current_line[..prefix_start];
		let after_cursor = &current_line[cursor_col.min(current_line.len())..];

		// Slash command completion
		let is_slash_command =
			prefix.starts_with('/') && before_prefix.trim().is_empty() && !prefix[1..].contains('/');

		if is_slash_command {
			let new_line = format!("{before_prefix}/{} {after_cursor}", item.value);
			let mut new_lines = lines.to_vec();
			new_lines[cursor_line] = new_line;
			return CompletionResult {
				lines: new_lines,
				cursor_line,
				cursor_col: before_prefix.len() + item.value.len() + 2,
			};
		}

		// @ file attachment completion
		if prefix.starts_with('@') {
			let new_line = format!("{before_prefix}{} {after_cursor}", item.value);
			let mut new_lines = lines.to_vec();
			new_lines[cursor_line] = new_line;
			return CompletionResult {
				lines: new_lines,
				cursor_line,
				cursor_col: before_prefix.len() + item.value.len() + 1,
			};
		}

		// Slash command argument or file path
		let new_line = format!("{before_prefix}{}{after_cursor}", item.value);
		let mut new_lines = lines.to_vec();
		new_lines[cursor_line] = new_line;
		CompletionResult {
			lines: new_lines,
			cursor_line,
			cursor_col: before_prefix.len() + item.value.len(),
		}
	}

	fn get_inline_hint(
		&self,
		lines: &[String],
		cursor_line: usize,
		cursor_col: usize,
	) -> Option<String> {
		let current_line = lines.get(cursor_line).map_or("", String::as_str);
		let text_before_cursor = &current_line[..cursor_col.min(current_line.len())];

		if !text_before_cursor.starts_with('/') {
			return None;
		}

		let space_index = text_before_cursor.find(' ')?;
		let command_name = &text_before_cursor[1..space_index];
		let argument_text = &text_before_cursor[space_index + 1..];

		let command = self
			.commands
			.iter()
			.find(|cmd| cmd.name() == command_name)?;

		if let CommandEntry::Slash(slash_cmd) = command
			&& let Some(ref get_hint) = slash_cmd.get_inline_hint
		{
			return get_hint(argument_text);
		}

		None
	}
}

impl CombinedAutocompleteProvider {
	fn get_slash_command_suggestions(&self, text_before_cursor: &str) -> Option<SuggestionResult> {
		if let Some(space_idx) = text_before_cursor.find(' ') {
			// Space found — complete command arguments
			let command_name = &text_before_cursor[1..space_idx];
			let argument_text = &text_before_cursor[space_idx + 1..];

			let command = self
				.commands
				.iter()
				.find(|cmd| cmd.name() == command_name)?;

			if let CommandEntry::Slash(slash_cmd) = command
				&& let Some(ref get_completions) = slash_cmd.get_argument_completions
			{
				let suggestions = get_completions(argument_text)?;
				if suggestions.is_empty() {
					return None;
				}
				return Some(SuggestionResult {
					items:  suggestions,
					prefix: argument_text.to_owned(),
				});
			}

			None
		} else {
			// No space yet — complete command names
			let prefix = &text_before_cursor[1..]; // Remove "/"
			let lower_prefix = prefix.to_lowercase();

			let mut matches: Vec<(AutocompleteItem, u32)> = self
				.commands
				.iter()
				.filter_map(|cmd| {
					let name = cmd.name();
					let lower_name = name.to_lowercase();
					let lower_desc = cmd.description().unwrap_or("").to_lowercase();

					let name_matches = fuzzy_match_subsequence(&lower_prefix, &lower_name);
					let desc_matches = fuzzy_match_subsequence(&lower_prefix, &lower_desc);

					if !name_matches && !desc_matches {
						return None;
					}

					let name_score = if name_matches {
						fuzzy_score(&lower_prefix, &lower_name)
					} else {
						0
					};
					let desc_score = if desc_matches {
						fuzzy_score(&lower_prefix, &lower_desc) / 2
					} else {
						0
					};
					let score = name_score.max(desc_score);

					let mut item = AutocompleteItem::new(name, cmd.label());
					item.description = cmd.description().map(ToOwned::to_owned);

					Some((item, score))
				})
				.collect();

			if matches.is_empty() {
				return None;
			}

			matches.sort_by_key(|m| Reverse(m.1));

			Some(SuggestionResult {
				items:  matches.into_iter().map(|(item, _)| item).collect(),
				prefix: text_before_cursor.to_owned(),
			})
		}
	}
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::*;

	// ── Pure helper tests ────────────────────────────────────────────

	#[test]
	fn test_fuzzy_match_subsequence_basic() {
		assert!(fuzzy_match_subsequence("", "anything"));
		assert!(fuzzy_match_subsequence("abc", "abc"));
		assert!(fuzzy_match_subsequence("ac", "abc"));
		assert!(fuzzy_match_subsequence("wig", "skill:wig"));
		assert!(!fuzzy_match_subsequence("xyz", "abc"));
		assert!(!fuzzy_match_subsequence("toolong", "hi"));
	}

	#[test]
	fn test_fuzzy_score_ranking() {
		let exact = fuzzy_score("hello", "hello");
		let starts = fuzzy_score("hel", "hello");
		let contains = fuzzy_score("ell", "hello");
		let subseq = fuzzy_score("hlo", "hello");

		assert!(exact > starts);
		assert!(starts > contains);
		assert!(contains > subseq);
	}

	#[test]
	fn test_fuzzy_score_no_match() {
		assert_eq!(fuzzy_score("xyz", "abc"), 0);
	}

	#[test]
	fn test_find_last_delimiter() {
		assert_eq!(find_last_delimiter("hello world"), Some(5));
		assert_eq!(find_last_delimiter("hello"), None);
		assert_eq!(find_last_delimiter("a=\"b"), Some(2));
	}

	#[test]
	fn test_find_unclosed_quote_start() {
		assert_eq!(find_unclosed_quote_start("hello"), None);
		assert_eq!(find_unclosed_quote_start("hello \"world"), Some(6));
		assert_eq!(find_unclosed_quote_start("\"hello\" world"), None);
		assert_eq!(find_unclosed_quote_start("\"hello\" \"world"), Some(8));
	}

	#[test]
	fn test_parse_path_prefix() {
		let p = parse_path_prefix("hello");
		assert_eq!(p.raw, "hello");
		assert!(!p.is_at);
		assert!(!p.is_quoted);

		let p = parse_path_prefix("@hello");
		assert_eq!(p.raw, "hello");
		assert!(p.is_at);

		let p = parse_path_prefix("@\"hello");
		assert_eq!(p.raw, "hello");
		assert!(p.is_at);
		assert!(p.is_quoted);

		let p = parse_path_prefix("\"hello");
		assert_eq!(p.raw, "hello");
		assert!(!p.is_at);
		assert!(p.is_quoted);
	}

	#[test]
	fn test_build_completion_value() {
		assert_eq!(build_completion_value("src/", true, false, false), "src/");
		assert_eq!(build_completion_value("file.rs", false, true, false), "@file.rs");
		assert_eq!(build_completion_value("my file.rs", false, false, false), "\"my file.rs\"");
		assert_eq!(build_completion_value("my dir/", true, true, true), "@\"my dir/");
	}

	#[test]
	fn test_extract_path_prefix_natural() {
		assert_eq!(
			CombinedAutocompleteProvider::extract_path_prefix("hello src/", false),
			Some("src/".to_owned())
		);
		assert_eq!(CombinedAutocompleteProvider::extract_path_prefix("hello", false), None);
		assert_eq!(
			CombinedAutocompleteProvider::extract_path_prefix("./foo", false),
			Some("./foo".to_owned())
		);
	}

	#[test]
	fn test_extract_path_prefix_forced() {
		assert_eq!(
			CombinedAutocompleteProvider::extract_path_prefix("hello", true),
			Some("hello".to_owned())
		);
		assert_eq!(
			CombinedAutocompleteProvider::extract_path_prefix("hey /", true),
			Some("/".to_owned())
		);
	}

	// ── Slash command tests ──────────────────────────────────────────

	#[test]
	fn test_slash_command_completion() {
		let commands = vec![
			CommandEntry::Item(
				AutocompleteItem::new("model", "model").with_description("Change model"),
			),
			CommandEntry::Item(AutocompleteItem::new("help", "help").with_description("Show help")),
			CommandEntry::Item(AutocompleteItem::new("clear", "clear")),
		];
		let mut provider = CombinedAutocompleteProvider::new(commands, PathBuf::from("/tmp"));

		let result = provider.get_suggestions(&["/mo".to_owned()], 0, 3);
		assert!(result.is_some());
		let result = result.unwrap();
		assert!(!result.items.is_empty());
		assert_eq!(result.items[0].value, "model");
	}

	#[test]
	fn test_slash_command_fuzzy() {
		let commands = vec![
			CommandEntry::Item(AutocompleteItem::new("model", "model")),
			CommandEntry::Item(AutocompleteItem::new("help", "help")),
		];
		let mut provider = CombinedAutocompleteProvider::new(commands, PathBuf::from("/tmp"));

		// "ml" is a subsequence of "model"
		let result = provider.get_suggestions(&["/ml".to_owned()], 0, 3);
		assert!(result.is_some());
		let result = result.unwrap();
		assert!(result.items.iter().any(|i| i.value == "model"));
	}

	#[test]
	fn test_apply_completion_slash_command() {
		let commands = vec![CommandEntry::Item(AutocompleteItem::new("model", "model"))];
		let provider = CombinedAutocompleteProvider::new(commands, PathBuf::from("/tmp"));

		let item = AutocompleteItem::new("model", "model");
		let result = provider.apply_completion(&["/mo".to_owned()], 0, 3, &item, "/mo");
		assert_eq!(result.lines[0], "/model ");
		assert_eq!(result.cursor_col, 7);
	}

	#[test]
	fn test_apply_completion_at_prefix() {
		let provider = CombinedAutocompleteProvider::new(Vec::new(), PathBuf::from("/tmp"));

		let item = AutocompleteItem::new("@src/main.rs", "main.rs");
		let result = provider.apply_completion(&["@sr".to_owned()], 0, 3, &item, "@sr");
		assert_eq!(result.lines[0], "@src/main.rs ");
		assert_eq!(result.cursor_col, 13);
	}

	// ── File suggestion tests (require temp dir) ─────────────────────

	#[test]
	fn test_file_suggestions_basic() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::write(base.join("foo.rs"), "").unwrap();
		fs::write(base.join("bar.rs"), "").unwrap();
		fs::create_dir(base.join("src")).unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());
		let suggestions = provider.get_file_suggestions("");
		assert!(suggestions.len() >= 3);
		assert!(suggestions.iter().any(|s| s.label == "src/"));
		assert!(suggestions.iter().any(|s| s.label == "foo.rs"));
	}

	#[test]
	fn test_file_suggestions_prefix_filter() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::write(base.join("foo.rs"), "").unwrap();
		fs::write(base.join("bar.rs"), "").unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());
		let suggestions = provider.get_file_suggestions("fo");
		assert_eq!(suggestions.len(), 1);
		assert_eq!(suggestions[0].label, "foo.rs");
	}

	#[test]
	fn test_file_suggestions_excludes_git() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::create_dir(base.join(".git")).unwrap();
		fs::create_dir(base.join(".github")).unwrap();
		fs::write(base.join("file.rs"), "").unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());
		let suggestions = provider.get_file_suggestions("");
		assert!(suggestions.iter().any(|s| s.label == ".github/"));
		assert!(
			!suggestions
				.iter()
				.any(|s| s.label == ".git/" || s.label == ".git")
		);
	}

	#[test]
	fn test_file_suggestions_directories_first() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::write(base.join("aaa.rs"), "").unwrap();
		fs::create_dir(base.join("bbb")).unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());
		let suggestions = provider.get_file_suggestions("");
		// Directory should come first despite alphabetical order
		assert_eq!(suggestions[0].label, "bbb/");
	}

	#[test]
	fn test_fuzzy_file_suggestions() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::write(base.join("history-search.ts"), "export const x = 1;\n").unwrap();

		let provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());
		let suggestions = provider.get_fuzzy_file_suggestions("histsr", false);
		let values: Vec<&str> = suggestions.iter().map(|s| s.value.as_str()).collect();
		assert!(values.contains(&"@history-search.ts"));
	}

	#[test]
	fn test_fuzzy_file_excludes_git() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::create_dir(base.join(".github")).unwrap();
		fs::create_dir(base.join(".git")).unwrap();
		fs::create_dir(base.join(".github").join("workflows")).unwrap();
		fs::write(base.join(".github").join("workflows").join("ci.yml"), "name: ci").unwrap();
		fs::write(base.join(".git").join("config"), "[core]").unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());

		// Using get_suggestions with "@"
		let result = provider.get_suggestions(&["@".to_owned()], 0, 1);
		if let Some(result) = result {
			let values: Vec<&str> = result.items.iter().map(|s| s.value.as_str()).collect();
			assert!(values.iter().any(|v| v.contains(".github")));
			assert!(
				!values
					.iter()
					.any(|v| *v == "@.git" || v.starts_with("@.git/"))
			);
		}
	}

	#[test]
	fn test_scoped_fuzzy_search() {
		let root_dir = tempfile::tempdir().unwrap();
		let root = root_dir.path();

		let cwd = root.join("cwd");
		let outside = root.join("outside");
		fs::create_dir_all(&cwd).unwrap();
		fs::create_dir_all(outside.join("nested").join("deeper")).unwrap();

		fs::write(cwd.join("alpha-local.ts"), "export const local = 1;\n").unwrap();
		fs::write(outside.join("nested").join("alpha.ts"), "export const alpha = 1;\n").unwrap();
		fs::write(
			outside.join("nested").join("deeper").join("also-alpha.ts"),
			"export const also = 1;\n",
		)
		.unwrap();
		fs::write(outside.join("nested").join("deeper").join("zzz.ts"), "export const zzz = 1;\n")
			.unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), cwd);
		let line = "@../outside/a".to_owned();
		let result = provider.get_suggestions(&[line.clone()], 0, line.len());

		assert!(result.is_some(), "should have results");
		let result = result.unwrap();
		let values: Vec<&str> = result.items.iter().map(|s| s.value.as_str()).collect();

		assert!(
			values.iter().any(|v| v.contains("alpha.ts")),
			"should find alpha.ts, got: {values:?}"
		);
		assert!(
			values.iter().any(|v| v.contains("also-alpha.ts")),
			"should find also-alpha.ts, got: {values:?}"
		);
		assert!(
			!values.iter().any(|v| v.contains("zzz.ts")),
			"should not find zzz.ts, got: {values:?}"
		);
		assert!(
			!values.iter().any(|v| v.contains("alpha-local.ts")),
			"should not find alpha-local.ts (in cwd, not outside), got: {values:?}"
		);
	}

	#[test]
	fn test_force_file_suggestions_slash_command() {
		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), PathBuf::from("/tmp"));

		// Should not trigger for slash commands
		let result = provider.get_force_file_suggestions(&["/model".to_owned()], 0, 6);
		assert!(result.is_none());
	}

	#[test]
	fn test_force_file_suggestions_after_command() {
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path();

		fs::write(base.join("file.txt"), "").unwrap();

		let mut provider = CombinedAutocompleteProvider::new(Vec::new(), base.to_path_buf());
		let result = provider.get_force_file_suggestions(&["/command /".to_owned()], 0, 10);
		// Should trigger because we're past the command name
		assert!(result.is_some());
	}

	#[test]
	fn test_inline_hint() {
		let commands = vec![CommandEntry::Slash(SlashCommand {
			name:                     "model".to_owned(),
			description:              Some("Change model".to_owned()),
			get_argument_completions: None,
			get_inline_hint:          Some(Box::new(|arg| {
				if arg.is_empty() {
					Some("sonnet".to_owned())
				} else {
					None
				}
			})),
		})];
		let provider = CombinedAutocompleteProvider::new(commands, PathBuf::from("/tmp"));

		let hint = provider.get_inline_hint(&["/model ".to_owned()], 0, 7);
		assert_eq!(hint, Some("sonnet".to_owned()));

		// No hint for non-slash
		let hint = provider.get_inline_hint(&["hello".to_owned()], 0, 5);
		assert!(hint.is_none());
	}

	#[test]
	fn test_should_trigger_file_completion() {
		assert!(CombinedAutocompleteProvider::should_trigger_file_completion(
			&["hello world".to_owned()],
			0,
			11
		));
		assert!(!CombinedAutocompleteProvider::should_trigger_file_completion(
			&["/model".to_owned()],
			0,
			6
		));
		assert!(CombinedAutocompleteProvider::should_trigger_file_completion(
			&["/model sonnet".to_owned()],
			0,
			13
		));
	}
}
