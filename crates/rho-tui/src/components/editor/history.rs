//! Prompt history for the editor.
//!
//! Provides up/down arrow history navigation with deduplication
//! and size limits.

/// Maximum number of history entries to keep.
const MAX_HISTORY: usize = 100;

/// Prompt history manager.
#[derive(Debug, Default)]
pub struct History {
	/// History entries, most recent first.
	entries: Vec<String>,
	/// Current browsing index: -1 = not browsing, 0 = most recent, etc.
	/// We use `Option<usize>` where `None` = not browsing.
	index:   Option<usize>,
}

impl History {
	pub fn new() -> Self {
		Self::default()
	}

	/// Load initial history from external storage.
	pub fn load(&mut self, entries: Vec<String>) {
		self.entries = entries;
		self.index = None;
	}

	/// Add a prompt to history (most recent first).
	/// Trims whitespace, skips empty strings, and deduplicates consecutive
	/// entries.
	pub fn add(&mut self, text: &str) {
		let trimmed = text.trim();
		if trimmed.is_empty() {
			return;
		}
		// Don't add consecutive duplicates
		if self.entries.first().is_some_and(|e| e == trimmed) {
			return;
		}
		self.entries.insert(0, trimmed.to_owned());
		if self.entries.len() > MAX_HISTORY {
			self.entries.pop();
		}
	}

	/// Whether we are currently browsing history.
	pub const fn is_browsing(&self) -> bool {
		self.index.is_some()
	}

	/// Get the current history entry, or None if not browsing.
	pub fn current_entry(&self) -> Option<&str> {
		self
			.index
			.and_then(|i| self.entries.get(i))
			.map(String::as_str)
	}

	/// Reset browsing state (exit history mode).
	pub const fn reset(&mut self) {
		self.index = None;
	}

	/// Navigate history. `direction`: -1 = older (Up), +1 = newer (Down).
	///
	/// Returns `Some(text)` if an entry should be shown, `None` if we exited
	/// history (returned to current editor state), or `Err(())` if the
	/// navigation was a no-op (e.g., already at oldest/not browsing).
	#[allow(clippy::result_unit_err, reason = "simple navigation API where Err(()) means no-op")]
	pub fn navigate(&mut self, direction: i32) -> Result<Option<String>, ()> {
		if self.entries.is_empty() {
			return Err(());
		}

		let current = self.index.map_or(-1_i64, |i| i as i64);
		// Up(-1) increases index (older), Down(+1) decreases (newer)
		let new_index = current - i64::from(direction);

		if new_index < -1 || new_index >= self.entries.len() as i64 {
			return Err(());
		}

		if new_index == -1 {
			self.index = None;
			Ok(None) // Return to current state
		} else {
			let idx = new_index as usize;
			self.index = Some(idx);
			Ok(Some(self.entries[idx].clone()))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_add_and_navigate() {
		let mut hist = History::new();
		hist.add("first");
		hist.add("second");
		hist.add("third");

		// Navigate up (older)
		assert_eq!(hist.navigate(-1), Ok(Some("third".into())));
		assert_eq!(hist.navigate(-1), Ok(Some("second".into())));
		assert_eq!(hist.navigate(-1), Ok(Some("first".into())));
		// Can't go further
		assert_eq!(hist.navigate(-1), Err(()));

		// Navigate back down (newer)
		assert_eq!(hist.navigate(1), Ok(Some("second".into())));
		assert_eq!(hist.navigate(1), Ok(Some("third".into())));
		// Return to current
		assert_eq!(hist.navigate(1), Ok(None));
	}

	#[test]
	fn test_empty_history() {
		let mut hist = History::new();
		assert_eq!(hist.navigate(-1), Err(()));
	}

	#[test]
	fn test_skip_empty_and_whitespace() {
		let mut hist = History::new();
		hist.add("");
		hist.add("   ");
		hist.add("valid");

		assert_eq!(hist.entries.len(), 1);
		assert_eq!(hist.navigate(-1), Ok(Some("valid".into())));
	}

	#[test]
	fn test_deduplicate_consecutive() {
		let mut hist = History::new();
		hist.add("same");
		hist.add("same");
		hist.add("same");
		assert_eq!(hist.entries.len(), 1);
	}

	#[test]
	fn test_allow_non_consecutive_duplicates() {
		let mut hist = History::new();
		hist.add("first");
		hist.add("second");
		hist.add("first");
		assert_eq!(hist.entries.len(), 3);
	}

	#[test]
	fn test_limit_size() {
		let mut hist = History::new();
		for i in 0..105 {
			hist.add(&format!("prompt {i}"));
		}
		assert_eq!(hist.entries.len(), MAX_HISTORY);
		// Most recent should be "prompt 104"
		assert_eq!(hist.entries[0], "prompt 104");
		// Oldest should be "prompt 5"
		assert_eq!(hist.entries[MAX_HISTORY - 1], "prompt 5");
	}

	#[test]
	fn test_reset() {
		let mut hist = History::new();
		hist.add("entry");
		hist.navigate(-1).unwrap();
		assert!(hist.is_browsing());
		hist.reset();
		assert!(!hist.is_browsing());
	}
}
