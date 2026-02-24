//! Strip ANSI escape sequences, remove control characters, and normalize line
//! endings.

use crate::ansi::ansi_seq_len_bytes;

/// Sanitize UTF-8 text: strip ANSI escapes, remove control chars, normalize
/// line endings.
///
/// Returns `None` if the text is already clean (caller can use original).
pub fn sanitize_text_str(text: &str) -> Option<String> {
	let bytes = text.as_bytes();
	let mut did_change = false;
	let mut out = String::new();
	let mut last = 0usize;
	let mut i = 0usize;
	let len = bytes.len();

	while i < len {
		let b = bytes[i];

		// Allow tab + newline
		if b == b'\t' || b == b'\n' {
			i += 1;
			continue;
		}

		let mut remove_len = if b == crate::ESC_U8
			&& let Some(seq_len) = ansi_seq_len_bytes(bytes, i)
		{
			seq_len
		} else {
			0usize
		};

		if remove_len == 0 {
			if b == b'\r' {
				remove_len = 1;
			} else if b <= 0x1f || b == 0x7f {
				// C0 + DEL (except tab/newline handled above)
				remove_len = 1;
			} else if (0x80..=0x9f).contains(&b) {
				// C1 controls when they appear as single bytes
				remove_len = 1;
			}
		}

		if remove_len == 0 {
			i += 1;
			continue;
		}

		if !did_change {
			did_change = true;
			out = String::with_capacity(len);
		}
		if last != i {
			out.push_str(&text[last..i]);
		}
		i += remove_len;
		last = i;
	}

	if !did_change {
		return None;
	}
	if last < len {
		out.push_str(&text[last..]);
	}
	Some(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_sanitize_clean_text() {
		assert!(sanitize_text_str("hello world").is_none());
	}

	#[test]
	fn test_sanitize_strips_ansi() {
		let result = sanitize_text_str("\x1b[31mhello\x1b[0m").unwrap();
		assert_eq!(result, "hello");
	}

	#[test]
	fn test_sanitize_preserves_tab_newline() {
		assert!(sanitize_text_str("hello\tworld\n").is_none());
	}

	#[test]
	fn test_sanitize_strips_cr() {
		let result = sanitize_text_str("hello\r\nworld").unwrap();
		assert_eq!(result, "hello\nworld");
	}

	#[test]
	fn test_sanitize_strips_control() {
		let result = sanitize_text_str("hello\x01world").unwrap();
		assert_eq!(result, "helloworld");
	}
}
