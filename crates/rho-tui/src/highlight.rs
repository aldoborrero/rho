//! Syntax highlighting using syntect with ANSI-colored output.
//!
//! Maps syntect scopes to 11 semantic categories:
//! comment, keyword, function, variable, string, number, type, operator,
//! punctuation, inserted, deleted.
//!
//! # Example
//! ```
//! use rho_tui::highlight::{HighlightColors, highlight_code};
//!
//! let colors = HighlightColors {
//! 	comment:     "\x1b[38;2;128;128;128m".into(),
//! 	keyword:     "\x1b[38;2;198;120;221m".into(),
//! 	function:    "\x1b[38;2;97;175;239m".into(),
//! 	variable:    "\x1b[38;2;224;108;117m".into(),
//! 	string:      "\x1b[38;2;152;195;121m".into(),
//! 	number:      "\x1b[38;2;209;154;102m".into(),
//! 	r#type:      "\x1b[38;2;229;192;123m".into(),
//! 	operator:    "\x1b[38;2;86;182;194m".into(),
//! 	punctuation: "\x1b[38;2;171;178;191m".into(),
//! 	inserted:    None,
//! 	deleted:     None,
//! };
//! let result = highlight_code("let x = 1;", Some("rust"), &colors);
//! assert!(result.contains("let"));
//! ```

use std::{cell::RefCell, collections::HashMap, sync::OnceLock};

use syntect::parsing::{ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxReference, SyntaxSet};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static SCOPE_MATCHERS: OnceLock<ScopeMatchers> = OnceLock::new();

thread_local! {
	 static SCOPE_COLOR_CACHE: RefCell<HashMap<Scope, usize>> = RefCell::new(HashMap::with_capacity(256));
}

fn get_syntax_set() -> &'static SyntaxSet {
	SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Theme colors for syntax highlighting.
/// Each color is an ANSI escape sequence (e.g., `\x1b[38;2;255;0;0m`).
#[derive(Debug, Clone)]
pub struct HighlightColors {
	pub comment:     String,
	pub keyword:     String,
	pub function:    String,
	pub variable:    String,
	pub string:      String,
	pub number:      String,
	pub r#type:      String,
	pub operator:    String,
	pub punctuation: String,
	pub inserted:    Option<String>,
	pub deleted:     Option<String>,
}

/// Pre-compiled scope patterns for fast matching.
struct ScopeMatchers {
	comment:               Scope,
	string:                Scope,
	constant_character:    Scope,
	meta_string:           Scope,
	constant_numeric:      Scope,
	constant_integer:      Scope,
	constant:              Scope,
	keyword:               Scope,
	storage_type:          Scope,
	storage_modifier:      Scope,
	entity_name_function:  Scope,
	support_function:      Scope,
	meta_function_call:    Scope,
	variable_function:     Scope,
	entity_name_type:      Scope,
	support_type:          Scope,
	support_class:         Scope,
	entity_name_class:     Scope,
	entity_name_struct:    Scope,
	entity_name_enum:      Scope,
	entity_name_interface: Scope,
	entity_name_trait:     Scope,
	keyword_operator:      Scope,
	punctuation_accessor:  Scope,
	punctuation:           Scope,
	variable:              Scope,
	entity_name:           Scope,
	meta_path:             Scope,
	markup_inserted:       Scope,
	markup_deleted:        Scope,
	meta_diff_header:      Scope,
	meta_diff_range:       Scope,
}

impl ScopeMatchers {
	fn new() -> Self {
		Self {
			comment:               Scope::new("comment").unwrap(),
			string:                Scope::new("string").unwrap(),
			constant_character:    Scope::new("constant.character").unwrap(),
			meta_string:           Scope::new("meta.string").unwrap(),
			constant_numeric:      Scope::new("constant.numeric").unwrap(),
			constant_integer:      Scope::new("constant.integer").unwrap(),
			constant:              Scope::new("constant").unwrap(),
			keyword:               Scope::new("keyword").unwrap(),
			storage_type:          Scope::new("storage.type").unwrap(),
			storage_modifier:      Scope::new("storage.modifier").unwrap(),
			entity_name_function:  Scope::new("entity.name.function").unwrap(),
			support_function:      Scope::new("support.function").unwrap(),
			meta_function_call:    Scope::new("meta.function-call").unwrap(),
			variable_function:     Scope::new("variable.function").unwrap(),
			entity_name_type:      Scope::new("entity.name.type").unwrap(),
			support_type:          Scope::new("support.type").unwrap(),
			support_class:         Scope::new("support.class").unwrap(),
			entity_name_class:     Scope::new("entity.name.class").unwrap(),
			entity_name_struct:    Scope::new("entity.name.struct").unwrap(),
			entity_name_enum:      Scope::new("entity.name.enum").unwrap(),
			entity_name_interface: Scope::new("entity.name.interface").unwrap(),
			entity_name_trait:     Scope::new("entity.name.trait").unwrap(),
			keyword_operator:      Scope::new("keyword.operator").unwrap(),
			punctuation_accessor:  Scope::new("punctuation.accessor").unwrap(),
			punctuation:           Scope::new("punctuation").unwrap(),
			variable:              Scope::new("variable").unwrap(),
			entity_name:           Scope::new("entity.name").unwrap(),
			meta_path:             Scope::new("meta.path").unwrap(),
			markup_inserted:       Scope::new("markup.inserted").unwrap(),
			markup_deleted:        Scope::new("markup.deleted").unwrap(),
			meta_diff_header:      Scope::new("meta.diff.header").unwrap(),
			meta_diff_range:       Scope::new("meta.diff.range").unwrap(),
		}
	}
}

fn get_scope_matchers() -> &'static ScopeMatchers {
	SCOPE_MATCHERS.get_or_init(ScopeMatchers::new)
}

/// Language alias mappings.
pub const LANG_ALIASES: &[(&[&str], &str)] = &[
	(&["ts", "tsx", "typescript", "js", "jsx", "javascript", "mjs", "cjs"], "JavaScript"),
	(&["py", "python"], "Python"),
	(&["rb", "ruby"], "Ruby"),
	(&["rs", "rust"], "Rust"),
	(&["go", "golang"], "Go"),
	(&["java"], "Java"),
	(&["kt", "kotlin"], "Java"),
	(&["swift"], "Objective-C"),
	(&["c", "h"], "C"),
	(&["cpp", "cc", "cxx", "c++", "hpp", "hxx", "hh"], "C++"),
	(&["cs", "csharp"], "C#"),
	(&["php"], "PHP"),
	(&["sh", "bash", "zsh", "shell"], "Bash"),
	(&["fish"], "Shell-Unix-Generic"),
	(&["ps1", "powershell"], "PowerShell"),
	(&["html", "htm"], "HTML"),
	(&["css"], "CSS"),
	(&["scss"], "SCSS"),
	(&["sass"], "Sass"),
	(&["less"], "LESS"),
	(&["json"], "JSON"),
	(&["yaml", "yml"], "YAML"),
	(&["toml"], "TOML"),
	(&["xml"], "XML"),
	(&["md", "markdown"], "Markdown"),
	(&["sql"], "SQL"),
	(&["lua"], "Lua"),
	(&["perl", "pl"], "Perl"),
	(&["r"], "R"),
	(&["scala"], "Scala"),
	(&["clj", "clojure"], "Clojure"),
	(&["ex", "exs", "elixir"], "Ruby"),
	(&["erl", "erlang"], "Erlang"),
	(&["hs", "haskell"], "Haskell"),
	(&["ml", "ocaml"], "OCaml"),
	(&["vim"], "VimL"),
	(&["graphql", "gql"], "GraphQL"),
	(&["proto", "protobuf"], "Protocol Buffers"),
	(&["tf", "hcl", "terraform"], "Terraform"),
	(&["dockerfile", "docker"], "Dockerfile"),
	(&["makefile", "make"], "Makefile"),
	(&["cmake"], "CMake"),
	(&["ini", "cfg", "conf", "config", "properties"], "INI"),
	(&["diff", "patch"], "Diff"),
	(&["gitignore", "gitattributes", "gitmodules"], "Git Ignore"),
];

#[inline]
fn find_alias(lang: &str) -> Option<&'static str> {
	LANG_ALIASES
		.iter()
		.find(|(aliases, _)| aliases.iter().any(|a| lang.eq_ignore_ascii_case(a)))
		.map(|(_, target)| *target)
}

#[inline]
fn is_known_alias(lang: &str) -> bool {
	LANG_ALIASES
		.iter()
		.any(|(aliases, _)| aliases.iter().any(|a| lang.eq_ignore_ascii_case(a)))
}

#[inline]
fn compute_scope_color(s: Scope) -> usize {
	let m = get_scope_matchers();

	if m.comment.is_prefix_of(s) {
		return 0;
	}
	if m.markup_inserted.is_prefix_of(s) {
		return 9;
	}
	if m.markup_deleted.is_prefix_of(s) {
		return 10;
	}
	if m.meta_diff_header.is_prefix_of(s) || m.meta_diff_range.is_prefix_of(s) {
		return 1;
	}
	if m.string.is_prefix_of(s)
		|| m.constant_character.is_prefix_of(s)
		|| m.meta_string.is_prefix_of(s)
	{
		return 4;
	}
	if m.constant_numeric.is_prefix_of(s) || m.constant_integer.is_prefix_of(s) {
		return 5;
	}
	if m.keyword.is_prefix_of(s)
		|| m.storage_type.is_prefix_of(s)
		|| m.storage_modifier.is_prefix_of(s)
	{
		return 1;
	}
	if m.entity_name_function.is_prefix_of(s)
		|| m.support_function.is_prefix_of(s)
		|| m.meta_function_call.is_prefix_of(s)
		|| m.variable_function.is_prefix_of(s)
	{
		return 2;
	}
	if m.entity_name_type.is_prefix_of(s)
		|| m.support_type.is_prefix_of(s)
		|| m.support_class.is_prefix_of(s)
		|| m.entity_name_class.is_prefix_of(s)
		|| m.entity_name_struct.is_prefix_of(s)
		|| m.entity_name_enum.is_prefix_of(s)
		|| m.entity_name_interface.is_prefix_of(s)
		|| m.entity_name_trait.is_prefix_of(s)
	{
		return 6;
	}
	if m.keyword_operator.is_prefix_of(s) || m.punctuation_accessor.is_prefix_of(s) {
		return 7;
	}
	if m.punctuation.is_prefix_of(s) {
		return 8;
	}
	if m.variable.is_prefix_of(s) || m.entity_name.is_prefix_of(s) || m.meta_path.is_prefix_of(s) {
		return 3;
	}
	if m.constant.is_prefix_of(s) {
		return 5;
	}

	usize::MAX
}

#[inline]
fn scope_to_color_index(scope: &ScopeStack) -> usize {
	SCOPE_COLOR_CACHE.with(|cache| {
		let mut cache = cache.borrow_mut();
		for s in scope.as_slice().iter().rev() {
			let color_idx = *cache.entry(*s).or_insert_with(|| compute_scope_color(*s));
			if color_idx != usize::MAX {
				return color_idx;
			}
		}
		usize::MAX
	})
}

/// Find the appropriate syntax for a language name.
pub fn find_syntax<'a>(ss: &'a SyntaxSet, lang: &str) -> Option<&'a SyntaxReference> {
	if let Some(syn) = ss.find_syntax_by_token(lang) {
		return Some(syn);
	}
	if let Some(syn) = ss.find_syntax_by_extension(lang) {
		return Some(syn);
	}
	let alias = find_alias(lang)?;
	ss.find_syntax_by_name(alias)
		.or_else(|| ss.find_syntax_by_token(alias))
}

/// Highlight code and return ANSI-colored output.
pub fn highlight_code(code: &str, lang: Option<&str>, colors: &HighlightColors) -> String {
	let inserted = colors.inserted.as_deref().unwrap_or("");
	let deleted = colors.deleted.as_deref().unwrap_or("");

	let palette = [
		colors.comment.as_str(),
		colors.keyword.as_str(),
		colors.function.as_str(),
		colors.variable.as_str(),
		colors.string.as_str(),
		colors.number.as_str(),
		colors.r#type.as_str(),
		colors.operator.as_str(),
		colors.punctuation.as_str(),
		inserted,
		deleted,
	];

	let ss = get_syntax_set();

	let syntax = match lang {
		Some(l) => find_syntax(ss, l),
		None => None,
	}
	.unwrap_or_else(|| ss.find_syntax_plain_text());

	let mut parse_state = ParseState::new(syntax);
	let mut scope_stack = ScopeStack::new();
	let mut result = String::with_capacity(code.len() * 2);

	for line in syntect::util::LinesWithEndings::from(code) {
		let Ok(ops) = parse_state.parse_line(line, ss) else {
			result.push_str(line);
			continue;
		};

		let mut prev_end = 0;
		for (offset, op) in ops {
			let offset = offset.min(line.len());

			if offset > prev_end {
				let text = &line[prev_end..offset];
				let color_idx = scope_to_color_index(&scope_stack);

				if color_idx < palette.len() && !palette[color_idx].is_empty() {
					result.push_str(palette[color_idx]);
					result.push_str(text);
					result.push_str("\x1b[39m");
				} else {
					result.push_str(text);
				}
			}
			prev_end = offset;

			match op {
				ScopeStackOp::Push(scope) => {
					scope_stack.push(scope);
				},
				ScopeStackOp::Pop(count) => {
					for _ in 0..count {
						scope_stack.pop();
					}
				},
				ScopeStackOp::Restore | ScopeStackOp::Clear(_) | ScopeStackOp::Noop => {},
			}
		}

		if prev_end < line.len() {
			let text = &line[prev_end..];
			let color_idx = scope_to_color_index(&scope_stack);

			if color_idx < palette.len() && !palette[color_idx].is_empty() {
				result.push_str(palette[color_idx]);
				result.push_str(text);
				result.push_str("\x1b[39m");
			} else {
				result.push_str(text);
			}
		}
	}

	result
}

/// Detect language identifier from a file path (by extension or basename).
///
/// Checks special basenames (Dockerfile, Makefile, dotfiles), then falls back
/// to extension lookup against [`LANG_ALIASES`] and syntect's built-in syntax
/// set.
pub fn language_from_path(path: &str) -> Option<&'static str> {
	let basename = path.rsplit('/').next().unwrap_or(path);
	let lower = basename.to_ascii_lowercase();

	// Special dotfiles / basenames
	if lower.starts_with(".env") {
		return Some("ini");
	}
	if matches!(lower.as_str(), ".gitignore" | ".gitattributes" | ".gitmodules") {
		return Some("gitignore");
	}
	if matches!(lower.as_str(), ".editorconfig" | ".npmrc") {
		return Some("ini");
	}
	if lower == "dockerfile" {
		return Some("dockerfile");
	}
	if lower == "makefile" {
		return Some("makefile");
	}

	// Extension-based lookup
	let ext = lower.rsplit('.').next()?;
	if ext == lower {
		// No extension found (rsplit returned the whole string)
		return None;
	}
	find_alias(ext).or_else(|| {
		let ss = get_syntax_set();
		ss.find_syntax_by_extension(ext).map(|s| {
			// Leak the name so we can return &'static str.
			// This is fine — there's a bounded number of syntect syntaxes.
			let name: &'static str = Box::leak(s.name.clone().into_boxed_str());
			name
		})
	})
}

/// Check if a language is supported for highlighting.
pub fn supports_language(lang: &str) -> bool {
	if is_known_alias(lang) {
		return true;
	}
	let ss = get_syntax_set();
	find_syntax(ss, lang).is_some()
}

/// Get list of supported languages.
pub fn get_supported_languages() -> Vec<String> {
	let ss = get_syntax_set();
	ss.syntaxes().iter().map(|s| s.name.clone()).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_colors() -> HighlightColors {
		HighlightColors {
			comment:     "\x1b[38;2;128;128;128m".into(),
			keyword:     "\x1b[38;2;198;120;221m".into(),
			function:    "\x1b[38;2;97;175;239m".into(),
			variable:    "\x1b[38;2;224;108;117m".into(),
			string:      "\x1b[38;2;152;195;121m".into(),
			number:      "\x1b[38;2;209;154;102m".into(),
			r#type:      "\x1b[38;2;229;192;123m".into(),
			operator:    "\x1b[38;2;86;182;194m".into(),
			punctuation: "\x1b[38;2;171;178;191m".into(),
			inserted:    None,
			deleted:     None,
		}
	}

	#[test]
	fn test_highlight_rust() {
		let result = highlight_code("let x = 42;\n", Some("rust"), &test_colors());
		assert!(result.contains("let"));
		assert!(result.contains("42"));
	}

	#[test]
	fn test_highlight_unknown_lang() {
		let result = highlight_code("hello world", Some("nonexistent"), &test_colors());
		assert_eq!(result, "hello world");
	}

	#[test]
	fn test_supports_language() {
		assert!(supports_language("rust"));
		assert!(supports_language("python"));
		assert!(supports_language("ts"));
		assert!(supports_language("JavaScript"));
	}

	#[test]
	fn test_language_from_path_common_extensions() {
		assert_eq!(language_from_path("src/main.rs"), Some("Rust"));
		assert_eq!(language_from_path("lib.py"), Some("Python"));
		assert_eq!(language_from_path("app.ts"), Some("JavaScript"));
		assert_eq!(language_from_path("index.html"), Some("HTML"));
		assert_eq!(language_from_path("style.css"), Some("CSS"));
		assert_eq!(language_from_path("config.json"), Some("JSON"));
		assert_eq!(language_from_path("data.yaml"), Some("YAML"));
		assert_eq!(language_from_path("Cargo.toml"), Some("TOML"));
	}

	#[test]
	fn test_language_from_path_special_basenames() {
		assert_eq!(language_from_path("Dockerfile"), Some("dockerfile"));
		assert_eq!(language_from_path("Makefile"), Some("makefile"));
		assert_eq!(language_from_path(".gitignore"), Some("gitignore"));
		assert_eq!(language_from_path(".gitattributes"), Some("gitignore"));
		assert_eq!(language_from_path(".env"), Some("ini"));
		assert_eq!(language_from_path(".env.local"), Some("ini"));
		assert_eq!(language_from_path(".editorconfig"), Some("ini"));
		assert_eq!(language_from_path(".npmrc"), Some("ini"));
	}

	#[test]
	fn test_language_from_path_full_paths() {
		assert_eq!(language_from_path("/home/user/project/src/main.rs"), Some("Rust"));
		assert_eq!(language_from_path("crates/rho/src/lib.rs"), Some("Rust"));
	}

	#[test]
	fn test_language_from_path_unknown() {
		assert!(language_from_path("file.unknownext123").is_none());
		assert!(language_from_path("noextension").is_none());
	}
}
