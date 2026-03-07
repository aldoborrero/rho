use std::{fmt::Write as _, path::Path};

use async_trait::async_trait;
use serde_json::{Value, json};
use similar::TextDiff;
use tokio::fs;
use tokio_util::sync::CancellationToken;
use unicode_normalization::UnicodeNormalization;

use super::{Concurrency, OnToolUpdate, Tool, ToolOutput};

// ── Helpers ─────────────────────────────────────────────────────────────

/// Strip UTF-8 BOM (U+FEFF) if present. Returns the content without BOM
/// and whether a BOM was found.
fn strip_bom(s: &str) -> (&str, bool) {
	if let Some(rest) = s.strip_prefix('\u{FEFF}') {
		(rest, true)
	} else {
		(s, false)
	}
}

/// Detect the dominant line ending in `content`. Returns `"\r\n"` if CRLF
/// is found first, otherwise `"\n"`.
fn detect_line_ending(content: &str) -> &'static str {
	for b in content.bytes() {
		if b == b'\r' {
			return "\r\n";
		}
		if b == b'\n' {
			return "\n";
		}
	}
	"\n"
}

/// Normalize all line endings to LF.
fn normalize_to_lf(text: &str) -> String {
	text.replace("\r\n", "\n")
}

/// Convert LF line endings to the target ending.
fn restore_line_endings(text: &str, ending: &str) -> String {
	if ending == "\r\n" {
		text.replace('\n', "\r\n")
	} else {
		text.to_owned()
	}
}

/// Returns `true` for Unicode space characters that should be normalized
/// to a regular ASCII space.
const fn is_special_unicode_space(c: char) -> bool {
	matches!(
		c,
		'\u{00A0}'    // NO-BREAK SPACE
		| '\u{2000}'  // EN QUAD
		| '\u{2001}'  // EM QUAD
		| '\u{2002}'  // EN SPACE
		| '\u{2003}'  // EM SPACE
		| '\u{2004}'  // THREE-PER-EM SPACE
		| '\u{2005}'  // FOUR-PER-EM SPACE
		| '\u{2006}'  // SIX-PER-EM SPACE
		| '\u{2007}'  // FIGURE SPACE
		| '\u{2008}'  // PUNCTUATION SPACE
		| '\u{2009}'  // THIN SPACE
		| '\u{200A}'  // HAIR SPACE
		| '\u{202F}'  // NARROW NO-BREAK SPACE
		| '\u{205F}'  // MEDIUM MATHEMATICAL SPACE
		| '\u{3000}' // IDEOGRAPHIC SPACE
	)
}

/// Normalize a single character: collapse special spaces, straighten
/// typographic quotes and dashes to their ASCII equivalents.
const fn normalize_char(c: char) -> char {
	if is_special_unicode_space(c) {
		return ' ';
	}
	match c {
		// Typographic quotes → ASCII
		'\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
		'\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
		// Dashes → ASCII hyphen
		'\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
		_ => c,
	}
}

/// Build a normalized version of `content` for fuzzy matching:
/// - Strip trailing whitespace per line
/// - Normalize Unicode characters (spaces, quotes, dashes)
fn build_normalized_content(content: &str) -> String {
	let mut result = String::with_capacity(content.len());
	for (i, line) in content.split('\n').enumerate() {
		if i > 0 {
			result.push('\n');
		}
		let trimmed = line.trim_end();
		for c in trimmed.chars() {
			result.push(normalize_char(c));
		}
	}
	result
}

/// Map a byte range found in normalized content back to the corresponding
/// byte range in the original content.
///
/// Returns `(start, len)` as byte offsets into `original`.
fn map_normalized_range_to_original(
	original: &str,
	normalized: &str,
	norm_start: usize,
	norm_len: usize,
) -> Option<(usize, usize)> {
	// Walk both strings in lockstep, tracking byte positions.
	let orig_chars: Vec<(usize, char)> = original.char_indices().collect();
	let norm_chars: Vec<(usize, char)> = normalized.char_indices().collect();

	let mut oi = 0; // index into orig_chars
	let mut ni = 0; // index into norm_chars

	let mut orig_start: Option<usize> = None;
	let mut orig_end: usize = 0;

	while oi < orig_chars.len() && ni < norm_chars.len() {
		let (ob, oc) = orig_chars[oi];
		let (nb, _nc) = norm_chars[ni];

		// Have we entered the matched region?
		if nb == norm_start && orig_start.is_none() {
			orig_start = Some(ob);
		}

		// Are we past the matched region?
		if nb >= norm_start + norm_len && orig_start.is_some() {
			break;
		}

		// Track the end position in original
		if orig_start.is_some() {
			orig_end = ob + oc.len_utf8();
		}

		// Advance both cursors, handling trailing-whitespace stripping
		// and character normalization mismatches.
		let norm_c = normalize_char(oc);
		let advance_ni = if norm_chars[ni].1 == norm_c {
			true
		} else if oc.is_whitespace() && (ni >= norm_chars.len() || norm_chars[ni].1 == '\n') {
			// Trailing whitespace was stripped in normalized version
			false
		} else if oc == '\r' {
			// CRLF → LF normalization: skip the CR
			false
		} else {
			// Characters should match after normalization
			true
		};
		oi += 1;
		if advance_ni {
			ni += 1;
		}
	}

	// If we exhausted norm_chars while still in the match region, track remaining
	// original chars
	if orig_start.is_some() && ni >= norm_chars.len() {
		// We reached the end of normalized content during the match
		// Consume any remaining trailing whitespace in original
		while oi < orig_chars.len() {
			let (ob, oc) = orig_chars[oi];
			if oc == '\r' || (oc.is_whitespace() && oc != '\n') {
				orig_end = ob + oc.len_utf8();
				oi += 1;
			} else {
				break;
			}
		}
	}

	orig_start.map(|start| (start, orig_end - start))
}

/// Result of fuzzy text matching.
enum FuzzyMatchResult {
	/// Exact match found at the given byte offset.
	Exact(usize),
	/// Fuzzy match: byte offset and length in the *original* content.
	Fuzzy(usize, usize),
	/// No match found.
	NotFound,
}

/// Attempt to find `old_text` in `content` using a hierarchical strategy:
/// 1. Exact match (fast path)
/// 2. Normalized match (trailing whitespace stripped, Unicode normalized)
/// 3. NFC/NFD Unicode normalization variants
fn fuzzy_find_text(content: &str, old_text: &str) -> FuzzyMatchResult {
	// Fast path: exact match
	if let Some(pos) = content.find(old_text) {
		return FuzzyMatchResult::Exact(pos);
	}

	let normalized_content = build_normalized_content(content);
	let normalized_old = build_normalized_content(old_text);

	// Try normalized match
	if let Some(result) = try_normalized_find(content, &normalized_content, &normalized_old) {
		return result;
	}

	// Try NFC normalization of old_text
	let nfc_old: String = normalized_old.nfc().collect();
	if nfc_old != normalized_old
		&& let Some(result) = try_normalized_find(content, &normalized_content, &nfc_old)
	{
		return result;
	}

	// Try NFD normalization of old_text
	let nfd_old: String = normalized_old.nfd().collect();
	if nfd_old != normalized_old
		&& nfd_old != nfc_old
		&& let Some(result) = try_normalized_find(content, &normalized_content, &nfd_old)
	{
		return result;
	}

	FuzzyMatchResult::NotFound
}

/// Helper: try to find `needle` in `normalized_content` and map back to
/// original.
fn try_normalized_find(
	original: &str,
	normalized_content: &str,
	needle: &str,
) -> Option<FuzzyMatchResult> {
	if let Some(norm_pos) = normalized_content.find(needle)
		&& let Some((orig_start, orig_len)) =
			map_normalized_range_to_original(original, normalized_content, norm_pos, needle.len())
	{
		return Some(FuzzyMatchResult::Fuzzy(orig_start, orig_len));
	}
	None
}

/// Count occurrences of `old_text` in `content`, using the same fuzzy
/// matching strategy. Returns the count.
fn count_occurrences(content: &str, old_text: &str) -> usize {
	// First try exact matches
	let exact_count = content.matches(old_text).count();
	if exact_count > 0 {
		return exact_count;
	}

	// Count normalized matches
	let normalized_content = build_normalized_content(content);
	let normalized_old = build_normalized_content(old_text);

	normalized_content.matches(&normalized_old).count()
}

/// Generate a contextual diff between old and new content.
fn generate_diff(old_content: &str, new_content: &str) -> String {
	let diff = TextDiff::from_lines(old_content, new_content);
	let mut output = String::new();

	for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
		if idx > 0 {
			output.push_str("...\n");
		}
		for op in group {
			for change in diff.iter_changes(op) {
				let lineno = change
					.old_index()
					.or_else(|| change.new_index())
					.map_or(0, |n| n + 1);
				let sign = match change.tag() {
					similar::ChangeTag::Delete => '-',
					similar::ChangeTag::Insert => '+',
					similar::ChangeTag::Equal => ' ',
				};
				let _ = write!(output, "{lineno:4} {sign} ");
				output.push_str(change.as_str().unwrap_or(""));
				if change.missing_newline() {
					output.push('\n');
				}
			}
		}
	}

	output
}

// ── EditTool ────────────────────────────────────────────────────────────

/// Tool that performs surgical find-and-replace edits in files with fuzzy
/// matching support.
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
	fn name(&self) -> &str {
		"edit"
	}

	fn description(&self) -> &str {
		include_str!("../prompts/tools/edit.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			"type": "object",
			"properties": {
				"file_path": {
					"type": "string",
					"description": "Path to the file to edit"
				},
				"old_string": {
					"type": "string",
					"description": "Text to find in the file (must be unique)"
				},
				"new_string": {
					"type": "string",
					"description": "Replacement text"
				}
			},
			"required": ["file_path", "old_string", "new_string"]
		})
	}

	fn concurrency(&self) -> Concurrency {
		Concurrency::Exclusive
	}

	async fn execute(
		&self,
		input: &Value,
		cwd: &Path,
		_cancel: &CancellationToken,
		_on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput> {
		let raw_path = input
			.get("file_path")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

		let old_string = input
			.get("old_string")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: old_string"))?;

		let new_string = input
			.get("new_string")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: new_string"))?;

		if old_string.is_empty() {
			return Ok(ToolOutput {
				content:  "old_string cannot be empty".to_owned(),
				is_error: true,
			});
		}

		let path = cwd.join(raw_path);

		// Read the file
		let raw_content = match fs::read_to_string(&path).await {
			Ok(c) => c,
			Err(e) => {
				return Ok(ToolOutput {
					content:  format!("Error reading {}: {e}", path.display()),
					is_error: true,
				});
			},
		};

		// Strip BOM for matching purposes
		let (content_no_bom, had_bom) = strip_bom(&raw_content);

		// Detect and normalize line endings
		let line_ending = detect_line_ending(content_no_bom);
		let lf_content = normalize_to_lf(content_no_bom);
		let lf_old = normalize_to_lf(old_string);
		let lf_new = normalize_to_lf(new_string);

		// Check for ambiguity (multiple matches)
		let occurrence_count = count_occurrences(&lf_content, &lf_old);
		if occurrence_count > 1 {
			return Ok(ToolOutput {
				content:  format!(
					"Found {occurrence_count} occurrences in {}. Provide more context to make it \
					 unique.",
					path.display()
				),
				is_error: true,
			});
		}

		// Find the text using fuzzy matching
		let (start, match_len) = match fuzzy_find_text(&lf_content, &lf_old) {
			FuzzyMatchResult::Exact(pos) => (pos, lf_old.len()),
			FuzzyMatchResult::Fuzzy(pos, len) => (pos, len),
			FuzzyMatchResult::NotFound => {
				return Ok(ToolOutput {
					content:  format!("Could not find the specified text in {}", path.display()),
					is_error: true,
				});
			},
		};

		// Perform the replacement in LF-normalized space
		let mut new_content = String::with_capacity(lf_content.len() - match_len + lf_new.len());
		new_content.push_str(&lf_content[..start]);
		new_content.push_str(&lf_new);
		new_content.push_str(&lf_content[start + match_len..]);

		// No-op check
		if new_content == lf_content {
			return Ok(ToolOutput {
				content:  "No changes made — replacement produced identical content".to_owned(),
				is_error: true,
			});
		}

		// Generate diff before restoring line endings
		let diff = generate_diff(&lf_content, &new_content);

		// Restore original line endings
		let final_content = restore_line_endings(&new_content, line_ending);

		// Restore BOM if the original had one
		let write_content = if had_bom {
			format!("\u{FEFF}{final_content}")
		} else {
			final_content
		};

		// Atomic write via tempfile
		let parent = path.parent().unwrap_or_else(|| Path::new("."));
		let temp = tempfile::NamedTempFile::new_in(parent)?;
		fs::write(temp.path(), &write_content).await?;
		temp.persist(&path)?;

		Ok(ToolOutput { content: diff, is_error: false })
	}
}

#[cfg(test)]
mod tests {
	use tokio_util::sync::CancellationToken;

	use super::*;

	async fn run_edit(
		dir: &Path,
		file_path: &str,
		old_string: &str,
		new_string: &str,
	) -> ToolOutput {
		let tool = EditTool;
		let ct = CancellationToken::new();
		tool
			.execute(
				&json!({
					"file_path": file_path,
					"old_string": old_string,
					"new_string": new_string,
				}),
				dir,
				&ct,
				None,
			)
			.await
			.unwrap()
	}

	#[tokio::test]
	async fn test_edit_exact_match() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "hello world\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "hello", "goodbye").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert_eq!(content, "goodbye world\n");
	}

	#[tokio::test]
	async fn test_edit_multiline() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "line1\nline2\nline3\n").unwrap();

		let result = run_edit(
			dir.path(),
			file.to_str().unwrap(),
			"line1\nline2",
			"replaced1\nreplaced2\nreplaced3",
		)
		.await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert_eq!(content, "replaced1\nreplaced2\nreplaced3\nline3\n");
	}

	#[tokio::test]
	async fn test_edit_not_found() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "hello world\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "nonexistent", "replacement").await;
		assert!(result.is_error);
		assert!(result.content.contains("Could not find"));
	}

	#[tokio::test]
	async fn test_edit_ambiguous() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "foo bar foo\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "foo", "baz").await;
		assert!(result.is_error);
		assert!(result.content.contains("occurrences"));
	}

	#[tokio::test]
	async fn test_edit_empty_old_string() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "hello\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "", "something").await;
		assert!(result.is_error);
		assert!(result.content.contains("old_string cannot be empty"));
	}

	#[tokio::test]
	async fn test_edit_no_op() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "hello world\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "hello", "hello").await;
		assert!(result.is_error);
		assert!(result.content.contains("No changes made"));
	}

	#[tokio::test]
	async fn test_edit_file_not_found() {
		let dir = tempfile::tempdir().unwrap();

		let result = run_edit(dir.path(), "nonexistent.txt", "hello", "world").await;
		assert!(result.is_error);
		assert!(result.content.contains("Error reading"));
	}

	#[tokio::test]
	async fn test_edit_fuzzy_trailing_whitespace() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		// File has trailing spaces on some lines
		std::fs::write(&file, "hello   \nworld\n").unwrap();

		// Search without trailing spaces
		let result =
			run_edit(dir.path(), file.to_str().unwrap(), "hello\nworld", "goodbye\nearth").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert_eq!(content, "goodbye\nearth\n");
	}

	#[tokio::test]
	async fn test_edit_fuzzy_smart_quotes() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		// File has curly quotes
		std::fs::write(&file, "say \u{201C}hello\u{201D}\n").unwrap();

		// Search with straight quotes
		let result =
			run_edit(dir.path(), file.to_str().unwrap(), "say \"hello\"", "say \"goodbye\"").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert!(content.contains("goodbye"));
	}

	#[tokio::test]
	async fn test_edit_fuzzy_unicode_dashes() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		// File has em-dash
		std::fs::write(&file, "foo \u{2014} bar\n").unwrap();

		// Search with ASCII hyphen
		let result = run_edit(dir.path(), file.to_str().unwrap(), "foo - bar", "foo + bar").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert!(content.contains("foo + bar"));
	}

	#[tokio::test]
	async fn test_edit_fuzzy_unicode_spaces() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		// File has non-breaking space
		std::fs::write(&file, "hello\u{00A0}world\n").unwrap();

		// Search with regular space
		let result = run_edit(dir.path(), file.to_str().unwrap(), "hello world", "hello_world").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert_eq!(content, "hello_world\n");
	}

	#[tokio::test]
	async fn test_edit_crlf_preservation() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		// File uses CRLF
		std::fs::write(&file, "line1\r\nline2\r\nline3\r\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "line2", "replaced").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert_eq!(content, "line1\r\nreplaced\r\nline3\r\n");
	}

	#[tokio::test]
	async fn test_edit_bom_preservation() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		// File has UTF-8 BOM
		std::fs::write(&file, "\u{FEFF}hello world\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "hello", "goodbye").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);

		let content = std::fs::read_to_string(&file).unwrap();
		assert!(content.starts_with('\u{FEFF}'));
		assert!(content.contains("goodbye world"));
	}

	#[tokio::test]
	async fn test_edit_diff_output() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("test.txt");
		std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

		let result = run_edit(dir.path(), file.to_str().unwrap(), "beta", "BETA").await;
		assert!(!result.is_error, "unexpected error: {}", result.content);
		assert!(result.content.contains('-'), "diff should contain removal marker");
		assert!(result.content.contains('+'), "diff should contain addition marker");
	}
}
