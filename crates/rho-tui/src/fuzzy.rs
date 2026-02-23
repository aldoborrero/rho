//! Fuzzy matching utilities.
//!
//! Matches if all query characters appear in order (not necessarily
//! consecutive). Lower score = better match.

/// Result of a fuzzy match attempt.
#[derive(Debug, Clone, Copy)]
pub struct FuzzyMatch {
	pub matches: bool,
	pub score:   f64,
}

const ALPHANUMERIC_SWAP_PENALTY: f64 = 5.0;

const fn is_word_boundary(c: u8) -> bool {
	matches!(c, b' ' | b'\t' | b'-' | b'_' | b'.' | b'/' | b':')
}

fn score_match(query: &[u8], text: &[u8]) -> FuzzyMatch {
	if query.is_empty() {
		return FuzzyMatch { matches: true, score: 0.0 };
	}

	if query.len() > text.len() {
		return FuzzyMatch { matches: false, score: 0.0 };
	}

	let mut query_index = 0;
	let mut score: f64 = 0.0;
	// Use isize to match TS behavior where lastMatchIndex starts at -1
	let mut last_match_index: isize = -1;
	let mut consecutive_matches: i32 = 0;

	for i in 0..text.len() {
		if query_index >= query.len() {
			break;
		}
		if text[i] == query[query_index] {
			let is_boundary = i == 0 || is_word_boundary(text[i - 1]);
			let i_signed = i as isize;

			// Reward consecutive matches
			if last_match_index == i_signed - 1 {
				consecutive_matches += 1;
				score -= f64::from(consecutive_matches) * 5.0;
			} else {
				consecutive_matches = 0;
				// Penalize gaps
				if last_match_index >= 0 {
					score += (i_signed - last_match_index - 1) as f64 * 2.0;
				}
			}

			// Reward word boundary matches
			if is_boundary {
				score -= 10.0;
			}

			// Slight penalty for later matches
			score += i as f64 * 0.1;

			last_match_index = i_signed;
			query_index += 1;
		}
	}

	if query_index < query.len() {
		return FuzzyMatch { matches: false, score: 0.0 };
	}

	FuzzyMatch { matches: true, score }
}

fn build_alphanumeric_swap_queries(query: &[u8]) -> Vec<Vec<u8>> {
	let mut variants = Vec::new();

	for i in 0..query.len().saturating_sub(1) {
		let current = query[i];
		let next = query[i + 1];
		let is_alpha_num_swap = (current.is_ascii_lowercase() && next.is_ascii_digit())
			|| (current.is_ascii_digit() && next.is_ascii_lowercase());
		if !is_alpha_num_swap {
			continue;
		}
		let mut swapped = query.to_vec();
		swapped.swap(i, i + 1);
		// Deduplicate
		if !variants.contains(&swapped) {
			variants.push(swapped);
		}
	}

	variants
}

/// Fuzzy match a query against text. Lower score = better match.
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
	let query_lower = query.to_ascii_lowercase();
	let text_lower = text.to_ascii_lowercase();
	let query_bytes = query_lower.as_bytes();
	let text_bytes = text_lower.as_bytes();

	let direct = score_match(query_bytes, text_bytes);
	if direct.matches {
		return direct;
	}

	let mut best_swap: Option<FuzzyMatch> = None;
	for variant in build_alphanumeric_swap_queries(query_bytes) {
		let m = score_match(&variant, text_bytes);
		if !m.matches {
			continue;
		}
		let score = m.score + ALPHANUMERIC_SWAP_PENALTY;
		if best_swap.is_none() || score < best_swap.unwrap().score {
			best_swap = Some(FuzzyMatch { matches: true, score });
		}
	}

	best_swap.unwrap_or(direct)
}

/// Filter and sort items by fuzzy match quality (best matches first).
/// Supports space-separated tokens: all tokens must match.
pub fn fuzzy_filter<'a, T, F>(items: &'a [T], query: &str, get_text: F) -> Vec<&'a T>
where
	F: Fn(&T) -> &str,
{
	let trimmed = query.trim();
	if trimmed.is_empty() {
		return items.iter().collect();
	}

	let tokens: Vec<&str> = trimmed.split_whitespace().collect();
	if tokens.is_empty() {
		return items.iter().collect();
	}

	let mut results: Vec<(&T, f64)> = Vec::new();

	for item in items {
		let text = get_text(item);
		let mut total_score = 0.0;
		let mut all_match = true;

		for &token in &tokens {
			let m = fuzzy_match(token, text);
			if m.matches {
				total_score += m.score;
			} else {
				all_match = false;
				break;
			}
		}

		if all_match {
			results.push((item, total_score));
		}
	}

	results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
	results.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_empty_query_matches_everything() {
		let m = fuzzy_match("", "anything");
		assert!(m.matches);
		assert!((m.score - 0.0).abs() < f64::EPSILON);
	}

	#[test]
	fn test_exact_match() {
		let m = fuzzy_match("hello", "hello");
		assert!(m.matches);
	}

	#[test]
	fn test_subsequence_match() {
		let m = fuzzy_match("hlo", "hello");
		assert!(m.matches);
	}

	#[test]
	fn test_no_match() {
		let m = fuzzy_match("xyz", "hello");
		assert!(!m.matches);
	}

	#[test]
	fn test_query_longer_than_text() {
		let m = fuzzy_match("toolong", "hi");
		assert!(!m.matches);
	}

	#[test]
	fn test_case_insensitive() {
		let m = fuzzy_match("HeLLo", "hello world");
		assert!(m.matches);
	}

	#[test]
	fn test_word_boundary_bonus() {
		let at_boundary = fuzzy_match("fb", "foo-bar");
		let not_boundary = fuzzy_match("fb", "fxoxb");
		assert!(at_boundary.matches);
		assert!(not_boundary.matches);
		assert!(at_boundary.score < not_boundary.score);
	}

	#[test]
	fn test_consecutive_bonus() {
		let consecutive = fuzzy_match("hel", "hello");
		let spread = fuzzy_match("hel", "h_e_l");
		assert!(consecutive.matches);
		assert!(spread.matches);
		assert!(consecutive.score < spread.score);
	}

	#[test]
	fn test_alphanumeric_swap() {
		// "3p" should match "p3" via swap tolerance (3↔p swap)
		let m = fuzzy_match("3p", "p3");
		assert!(m.matches);
	}

	#[test]
	fn test_fuzzy_filter() {
		let items = vec!["foo-bar", "foobar", "baz", "fb"];
		let filtered = fuzzy_filter(&items, "fb", |s| s);
		assert!(filtered.contains(&&"foo-bar"));
		assert!(filtered.contains(&&"foobar"));
		assert!(filtered.contains(&&"fb"));
		assert!(!filtered.contains(&&"baz"));
	}

	#[test]
	fn test_fuzzy_filter_multi_token() {
		let items = vec!["src/foo/bar.rs", "src/baz.rs", "test/foo.rs"];
		let filtered = fuzzy_filter(&items, "src bar", |s| s);
		// Both "src/foo/bar.rs" and "src/baz.rs" match (b-a-r subsequence in baz.rs)
		assert_eq!(filtered.len(), 2);
		assert!(filtered.contains(&&"src/foo/bar.rs"));
		// "test/foo.rs" doesn't match "src"
		assert!(!filtered.contains(&&"test/foo.rs"));
	}

	#[test]
	fn test_fuzzy_filter_empty_query() {
		let items = vec!["a", "b", "c"];
		let filtered = fuzzy_filter(&items, "", |s| s);
		assert_eq!(filtered.len(), 3);
	}
}
