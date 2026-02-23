//! Emacs-style kill ring for cut/yank operations.
//!
//! Tracks killed (deleted) text entries. Consecutive kills can accumulate
//! into a single entry. Supports yank (paste most recent) and yank-pop
//! (cycle through older entries).

/// Ring buffer for Emacs-style kill/yank operations.
#[derive(Debug, Default)]
pub struct KillRing {
	ring: Vec<String>,
}

impl KillRing {
	pub fn new() -> Self {
		Self::default()
	}

	/// Add text to the kill ring.
	///
	/// - `prepend`: If accumulating, prepend (backward deletion) or append
	///   (forward deletion).
	/// - `accumulate`: Merge with the most recent entry instead of creating a
	///   new one.
	pub fn push(&mut self, text: &str, prepend: bool, accumulate: bool) {
		if text.is_empty() {
			return;
		}

		if accumulate && let Some(last) = self.ring.last_mut() {
			if prepend {
				let mut new = String::with_capacity(text.len() + last.len());
				new.push_str(text);
				new.push_str(last);
				*last = new;
			} else {
				last.push_str(text);
			}
			return;
		}

		self.ring.push(text.to_owned());
	}

	/// Get most recent entry without modifying the ring.
	pub fn peek(&self) -> Option<&str> {
		self.ring.last().map(String::as_str)
	}

	/// Move last entry to front (for yank-pop cycling).
	pub fn rotate(&mut self) {
		if self.ring.len() > 1
			&& let Some(last) = self.ring.pop()
		{
			self.ring.insert(0, last);
		}
	}

	pub const fn len(&self) -> usize {
		self.ring.len()
	}

	pub const fn is_empty(&self) -> bool {
		self.ring.is_empty()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_push_and_peek() {
		let mut kr = KillRing::new();
		assert!(kr.is_empty());
		assert_eq!(kr.peek(), None);

		kr.push("hello", false, false);
		assert_eq!(kr.peek(), Some("hello"));
		assert_eq!(kr.len(), 1);
	}

	#[test]
	fn test_push_empty_ignored() {
		let mut kr = KillRing::new();
		kr.push("", false, false);
		assert!(kr.is_empty());
	}

	#[test]
	fn test_accumulate_append() {
		let mut kr = KillRing::new();
		kr.push("hello", false, false);
		kr.push(" world", false, true);
		assert_eq!(kr.peek(), Some("hello world"));
		assert_eq!(kr.len(), 1);
	}

	#[test]
	fn test_accumulate_prepend() {
		let mut kr = KillRing::new();
		kr.push("world", false, false);
		kr.push("hello ", true, true);
		assert_eq!(kr.peek(), Some("hello world"));
		assert_eq!(kr.len(), 1);
	}

	#[test]
	fn test_accumulate_on_empty_creates_new() {
		let mut kr = KillRing::new();
		kr.push("hello", false, true);
		assert_eq!(kr.peek(), Some("hello"));
		assert_eq!(kr.len(), 1);
	}

	#[test]
	fn test_rotate() {
		let mut kr = KillRing::new();
		kr.push("a", false, false);
		kr.push("b", false, false);
		kr.push("c", false, false);

		assert_eq!(kr.peek(), Some("c"));
		kr.rotate();
		assert_eq!(kr.peek(), Some("b"));
		kr.rotate();
		assert_eq!(kr.peek(), Some("a"));
		kr.rotate();
		assert_eq!(kr.peek(), Some("c"));
	}

	#[test]
	fn test_rotate_single_element() {
		let mut kr = KillRing::new();
		kr.push("only", false, false);
		kr.rotate();
		assert_eq!(kr.peek(), Some("only"));
	}
}
