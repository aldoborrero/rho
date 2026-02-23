//! Stdin input buffer that splits raw terminal input into complete sequences.
//!
//! Necessary because stdin data events can arrive in partial chunks,
//! especially for escape sequences like mouse events. Without buffering,
//! partial sequences can be misinterpreted as regular keypresses.
//!
//! Based on code from `OpenTUI` (<https://github.com/anomalyco/opentui>)
//! MIT License - Copyright (c) 2025 opentui

const ESC: u8 = 0x1b;

const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// Result of sequence completeness check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeqStatus {
	Complete,
	Incomplete,
	NotEscape,
}

/// Check if data is a complete escape sequence or needs more data.
fn is_complete_sequence(data: &[u8]) -> SeqStatus {
	if data.is_empty() || data[0] != ESC {
		return SeqStatus::NotEscape;
	}

	if data.len() == 1 {
		return SeqStatus::Incomplete;
	}

	match data[1] {
		// CSI sequences: ESC [
		b'[' => {
			// Old-style mouse: ESC[M + 3 bytes = 6 total
			if data.len() >= 3 && data[2] == b'M' {
				return if data.len() >= 6 {
					SeqStatus::Complete
				} else {
					SeqStatus::Incomplete
				};
			}
			is_complete_csi(data)
		},
		// OSC sequences: ESC ]
		b']' => is_complete_osc(data),
		// DCS sequences: ESC P
		b'P' => is_complete_string_terminator(data),
		// APC sequences: ESC _
		b'_' => is_complete_string_terminator(data),
		// SS3 sequences: ESC O + single char
		b'O' => {
			if data.len() >= 3 {
				SeqStatus::Complete
			} else {
				SeqStatus::Incomplete
			}
		},
		// Meta key: ESC + single char
		_ => SeqStatus::Complete,
	}
}

/// CSI sequences: ESC [ ... final byte (0x40-0x7E).
fn is_complete_csi(data: &[u8]) -> SeqStatus {
	if data.len() < 3 {
		return SeqStatus::Incomplete;
	}

	let payload = &data[2..];
	let last = *payload.last().unwrap();

	// Final byte in 0x40..=0x7E range
	if (0x40..=0x7e).contains(&last) {
		// SGR mouse: ESC[<digits;digits;digits[Mm]
		if payload[0] == b'<' {
			if is_sgr_mouse_complete(payload) {
				return SeqStatus::Complete;
			}
			return SeqStatus::Incomplete;
		}
		return SeqStatus::Complete;
	}

	SeqStatus::Incomplete
}

/// Check if SGR mouse payload `<B;X;Y[Mm]` is complete.
fn is_sgr_mouse_complete(payload: &[u8]) -> bool {
	// Must end with M or m
	let last = *payload.last().unwrap();
	if last != b'M' && last != b'm' {
		return false;
	}
	// Strip < prefix and M/m suffix
	let inner = &payload[1..payload.len() - 1];
	// Must have exactly 3 semicolon-separated digit groups
	let mut parts = 0;
	let mut has_digit = false;
	for &b in inner {
		if b == b';' {
			if !has_digit {
				return false;
			}
			parts += 1;
			has_digit = false;
		} else if b.is_ascii_digit() {
			has_digit = true;
		} else {
			return false;
		}
	}
	if has_digit {
		parts += 1;
	}
	parts == 3
}

/// OSC sequences end with ST (ESC \) or BEL (\x07).
fn is_complete_osc(data: &[u8]) -> SeqStatus {
	if data.len() >= 2
		&& (*data.last().unwrap() == 0x07
			|| (data.len() >= 3 && data[data.len() - 2] == ESC && data[data.len() - 1] == b'\\'))
	{
		SeqStatus::Complete
	} else {
		SeqStatus::Incomplete
	}
}

/// DCS/APC sequences end with ST (ESC \).
fn is_complete_string_terminator(data: &[u8]) -> SeqStatus {
	if data.len() >= 4 && data[data.len() - 2] == ESC && data[data.len() - 1] == b'\\' {
		SeqStatus::Complete
	} else {
		SeqStatus::Incomplete
	}
}

/// Events emitted by the stdin buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdinEvent {
	/// A complete input sequence (single char or escape sequence).
	Data(String),
	/// Bracketed paste content (without the paste markers).
	Paste(String),
}

/// Extract complete sequences from a buffer, returning sequences and leftover.
fn extract_complete_sequences(buffer: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
	let mut sequences = Vec::new();
	let mut pos = 0;

	while pos < buffer.len() {
		if buffer[pos] == ESC {
			// Find end of this escape sequence
			let remaining = &buffer[pos..];
			let mut seq_end = 1;
			while seq_end <= remaining.len() {
				let candidate = &remaining[..seq_end];
				let status = is_complete_sequence(candidate);
				match status {
					SeqStatus::Complete => {
						sequences.push(candidate.to_vec());
						pos += seq_end;
						break;
					},
					SeqStatus::Incomplete => {
						seq_end += 1;
					},
					SeqStatus::NotEscape => {
						sequences.push(candidate.to_vec());
						pos += seq_end;
						break;
					},
				}
			}

			if seq_end > remaining.len() {
				return (sequences, remaining.to_vec());
			}
		} else {
			// Not an escape - take a single byte (or multi-byte UTF-8 char)
			let ch_len = utf8_char_len(buffer[pos]);
			let end = (pos + ch_len).min(buffer.len());
			sequences.push(buffer[pos..end].to_vec());
			pos = end;
		}
	}

	(sequences, Vec::new())
}

/// Get the length of a UTF-8 character from its first byte.
const fn utf8_char_len(b: u8) -> usize {
	if b < 0x80 {
		1
	} else if b < 0xe0 {
		2
	} else if b < 0xf0 {
		3
	} else {
		4
	}
}

/// Find a subsequence in a byte slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack.windows(needle.len()).position(|w| w == needle)
}

/// Buffers stdin input and produces complete sequences.
/// Handles partial escape sequences that arrive across multiple chunks.
#[derive(Debug)]
pub struct StdinBuffer {
	buffer:       Vec<u8>,
	paste_mode:   bool,
	paste_buffer: Vec<u8>,
}

impl StdinBuffer {
	pub const fn new() -> Self {
		Self { buffer: Vec::new(), paste_mode: false, paste_buffer: Vec::new() }
	}

	/// Process incoming data and return complete events.
	pub fn process(&mut self, data: &[u8]) -> Vec<StdinEvent> {
		if data.is_empty() && self.buffer.is_empty() {
			return vec![StdinEvent::Data(String::new())];
		}

		self.buffer.extend_from_slice(data);
		let mut events = Vec::new();

		if self.paste_mode {
			self.paste_buffer.append(&mut self.buffer);

			if let Some(end_idx) = find_subsequence(&self.paste_buffer, BRACKETED_PASTE_END) {
				let content = self.paste_buffer[..end_idx].to_vec();
				let remaining = self.paste_buffer[end_idx + BRACKETED_PASTE_END.len()..].to_vec();

				self.paste_mode = false;
				self.paste_buffer.clear();

				events.push(StdinEvent::Paste(String::from_utf8_lossy(&content).into_owned()));

				if !remaining.is_empty() {
					events.extend(self.process(&remaining));
				}
			}
			return events;
		}

		// Check for bracketed paste start
		if let Some(start_idx) = find_subsequence(&self.buffer, BRACKETED_PASTE_START) {
			// Emit anything before the paste start
			if start_idx > 0 {
				let before = self.buffer[..start_idx].to_vec();
				let (seqs, _remainder) = extract_complete_sequences(&before);
				for seq in seqs {
					events.push(StdinEvent::Data(String::from_utf8_lossy(&seq).into_owned()));
				}
			}

			let after_start = self.buffer[start_idx + BRACKETED_PASTE_START.len()..].to_vec();
			self.buffer.clear();
			self.paste_mode = true;
			self.paste_buffer = after_start;

			// Check if paste end is already in the buffer
			if let Some(end_idx) = find_subsequence(&self.paste_buffer, BRACKETED_PASTE_END) {
				let content = self.paste_buffer[..end_idx].to_vec();
				let remaining = self.paste_buffer[end_idx + BRACKETED_PASTE_END.len()..].to_vec();

				self.paste_mode = false;
				self.paste_buffer.clear();

				events.push(StdinEvent::Paste(String::from_utf8_lossy(&content).into_owned()));

				if !remaining.is_empty() {
					events.extend(self.process(&remaining));
				}
			}
			return events;
		}

		let buf = std::mem::take(&mut self.buffer);
		let (seqs, remainder) = extract_complete_sequences(&buf);
		self.buffer = remainder;

		for seq in seqs {
			events.push(StdinEvent::Data(String::from_utf8_lossy(&seq).into_owned()));
		}

		events
	}

	/// Flush any remaining buffered data as-is.
	pub fn flush(&mut self) -> Vec<String> {
		if self.buffer.is_empty() {
			return Vec::new();
		}

		let data = std::mem::take(&mut self.buffer);
		vec![String::from_utf8_lossy(&data).into_owned()]
	}

	/// Clear all buffered content without emitting.
	pub fn clear(&mut self) {
		self.buffer.clear();
		self.paste_mode = false;
		self.paste_buffer.clear();
	}

	/// Get current buffer contents (for testing/debugging).
	pub fn get_buffer(&self) -> &[u8] {
		&self.buffer
	}

	/// Check if there is pending incomplete data that would need a timeout
	/// flush.
	pub const fn has_pending(&self) -> bool {
		!self.buffer.is_empty()
	}
}

impl Default for StdinBuffer {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn collect_data(events: &[StdinEvent]) -> Vec<String> {
		events
			.iter()
			.filter_map(|e| match e {
				StdinEvent::Data(s) => Some(s.clone()),
				StdinEvent::Paste(_) => None,
			})
			.collect()
	}

	fn collect_paste(events: &[StdinEvent]) -> Vec<String> {
		events
			.iter()
			.filter_map(|e| match e {
				StdinEvent::Paste(s) => Some(s.clone()),
				StdinEvent::Data(_) => None,
			})
			.collect()
	}

	// ── Regular Characters ──────────────────────────────────────────

	#[test]
	fn regular_char_passthrough() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"a");
		assert_eq!(collect_data(&events), vec!["a"]);
	}

	#[test]
	fn multiple_regular_chars() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"abc");
		assert_eq!(collect_data(&events), vec!["a", "b", "c"]);
	}

	#[test]
	fn unicode_chars() {
		let mut buf = StdinBuffer::new();
		let input = "hello 世界";
		let events = buf.process(input.as_bytes());
		assert_eq!(collect_data(&events), vec!["h", "e", "l", "l", "o", " ", "世", "界"]);
	}

	// ── Complete Escape Sequences ───────────────────────────────────

	#[test]
	fn complete_mouse_sgr() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[<35;20;5m");
		assert_eq!(collect_data(&events), vec!["\x1b[<35;20;5m"]);
	}

	#[test]
	fn complete_arrow_key() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[A");
		assert_eq!(collect_data(&events), vec!["\x1b[A"]);
	}

	#[test]
	fn complete_function_key() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[11~");
		assert_eq!(collect_data(&events), vec!["\x1b[11~"]);
	}

	#[test]
	fn meta_key_sequence() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1ba");
		assert_eq!(collect_data(&events), vec!["\x1ba"]);
	}

	#[test]
	fn ss3_sequence() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1bOA");
		assert_eq!(collect_data(&events), vec!["\x1bOA"]);
	}

	// ── Partial Escape Sequences ────────────────────────────────────

	#[test]
	fn buffer_incomplete_mouse_sgr() {
		let mut buf = StdinBuffer::new();

		let events = buf.process(b"\x1b");
		assert!(collect_data(&events).is_empty());
		assert_eq!(buf.get_buffer(), b"\x1b");

		let events = buf.process(b"[<35");
		assert!(collect_data(&events).is_empty());
		assert_eq!(buf.get_buffer(), b"\x1b[<35");

		let events = buf.process(b";20;5m");
		assert_eq!(collect_data(&events), vec!["\x1b[<35;20;5m"]);
		assert!(buf.get_buffer().is_empty());
	}

	#[test]
	fn buffer_incomplete_csi() {
		let mut buf = StdinBuffer::new();

		let events = buf.process(b"\x1b[");
		assert!(collect_data(&events).is_empty());

		let events = buf.process(b"1;");
		assert!(collect_data(&events).is_empty());

		let events = buf.process(b"5H");
		assert_eq!(collect_data(&events), vec!["\x1b[1;5H"]);
	}

	#[test]
	fn buffer_split_many_chunks() {
		let mut buf = StdinBuffer::new();
		let chunks: &[&[u8]] = &[b"\x1b", b"[", b"<", b"3", b"5", b";", b"2", b"0", b";", b"5", b"m"];
		let mut all_events = Vec::new();
		for chunk in chunks {
			all_events.extend(buf.process(chunk));
		}
		assert_eq!(collect_data(&all_events), vec!["\x1b[<35;20;5m"]);
	}

	#[test]
	fn flush_incomplete_sequence() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[<35");
		assert!(collect_data(&events).is_empty());

		let flushed = buf.flush();
		assert_eq!(flushed, vec!["\x1b[<35"]);
	}

	// ── Mixed Content ───────────────────────────────────────────────

	#[test]
	fn chars_then_escape() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"abc\x1b[A");
		assert_eq!(collect_data(&events), vec!["a", "b", "c", "\x1b[A"]);
	}

	#[test]
	fn escape_then_chars() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[Aabc");
		assert_eq!(collect_data(&events), vec!["\x1b[A", "a", "b", "c"]);
	}

	#[test]
	fn multiple_complete_sequences() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[A\x1b[B\x1b[C");
		assert_eq!(collect_data(&events), vec!["\x1b[A", "\x1b[B", "\x1b[C"]);
	}

	#[test]
	fn partial_with_preceding_chars() {
		let mut buf = StdinBuffer::new();

		let events = buf.process(b"abc\x1b[<35");
		assert_eq!(collect_data(&events), vec!["a", "b", "c"]);
		assert_eq!(buf.get_buffer(), b"\x1b[<35");

		let events = buf.process(b";20;5m");
		assert_eq!(collect_data(&events), vec!["\x1b[<35;20;5m"]);
	}

	// ── Kitty Keyboard Protocol ─────────────────────────────────────

	#[test]
	fn kitty_csi_u_press() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[97u");
		assert_eq!(collect_data(&events), vec!["\x1b[97u"]);
	}

	#[test]
	fn kitty_csi_u_release() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[97;1:3u");
		assert_eq!(collect_data(&events), vec!["\x1b[97;1:3u"]);
	}

	#[test]
	fn batched_kitty_press_release() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[97u\x1b[97;1:3u");
		assert_eq!(collect_data(&events), vec!["\x1b[97u", "\x1b[97;1:3u"]);
	}

	#[test]
	fn multiple_batched_kitty() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[97u\x1b[97;1:3u\x1b[98u\x1b[98;1:3u");
		assert_eq!(collect_data(&events), vec![
			"\x1b[97u",
			"\x1b[97;1:3u",
			"\x1b[98u",
			"\x1b[98;1:3u"
		]);
	}

	#[test]
	fn kitty_arrow_with_event_type() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[1;1:1A");
		assert_eq!(collect_data(&events), vec!["\x1b[1;1:1A"]);
	}

	#[test]
	fn kitty_functional_key_release() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[3;1:3~");
		assert_eq!(collect_data(&events), vec!["\x1b[3;1:3~"]);
	}

	#[test]
	fn plain_char_with_kitty() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"a\x1b[97;1:3u");
		assert_eq!(collect_data(&events), vec!["a", "\x1b[97;1:3u"]);
	}

	#[test]
	fn kitty_then_plain_char() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[97ua");
		assert_eq!(collect_data(&events), vec!["\x1b[97u", "a"]);
	}

	#[test]
	fn rapid_kitty_typing() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[104u\x1b[104;1:3u\x1b[105u\x1b[105;1:3u");
		assert_eq!(collect_data(&events), vec![
			"\x1b[104u",
			"\x1b[104;1:3u",
			"\x1b[105u",
			"\x1b[105;1:3u"
		]);
	}

	// ── Mouse Events ────────────────────────────────────────────────

	#[test]
	fn mouse_press() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[<0;10;5M");
		assert_eq!(collect_data(&events), vec!["\x1b[<0;10;5M"]);
	}

	#[test]
	fn mouse_release() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[<0;10;5m");
		assert_eq!(collect_data(&events), vec!["\x1b[<0;10;5m"]);
	}

	#[test]
	fn mouse_move() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[<35;20;5m");
		assert_eq!(collect_data(&events), vec!["\x1b[<35;20;5m"]);
	}

	#[test]
	fn split_mouse_events() {
		let mut buf = StdinBuffer::new();
		let mut all = Vec::new();
		all.extend(buf.process(b"\x1b[<3"));
		all.extend(buf.process(b"5;1"));
		all.extend(buf.process(b"5;"));
		all.extend(buf.process(b"10m"));
		assert_eq!(collect_data(&all), vec!["\x1b[<35;15;10m"]);
	}

	#[test]
	fn multiple_mouse_events() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[<35;1;1m\x1b[<35;2;2m\x1b[<35;3;3m");
		assert_eq!(collect_data(&events), vec!["\x1b[<35;1;1m", "\x1b[<35;2;2m", "\x1b[<35;3;3m"]);
	}

	#[test]
	fn old_style_mouse() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b[M abc");
		assert_eq!(collect_data(&events), vec!["\x1b[M ab", "c"]);
	}

	#[test]
	fn buffer_incomplete_old_style_mouse() {
		let mut buf = StdinBuffer::new();

		let events = buf.process(b"\x1b[M");
		assert!(collect_data(&events).is_empty());
		assert_eq!(buf.get_buffer(), b"\x1b[M");

		let events = buf.process(b" a");
		assert!(collect_data(&events).is_empty());
		assert_eq!(buf.get_buffer(), b"\x1b[M a");

		let events = buf.process(b"b");
		assert_eq!(collect_data(&events), vec!["\x1b[M ab"]);
	}

	// ── Edge Cases ──────────────────────────────────────────────────

	#[test]
	fn empty_input() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"");
		assert_eq!(collect_data(&events), vec![""]);
	}

	#[test]
	fn lone_escape_flush() {
		let mut buf = StdinBuffer::new();
		let events = buf.process(b"\x1b");
		assert!(collect_data(&events).is_empty());

		let flushed = buf.flush();
		assert_eq!(flushed, vec!["\x1b"]);
	}

	#[test]
	fn very_long_sequence() {
		let mut buf = StdinBuffer::new();
		let mut seq = Vec::new();
		seq.extend_from_slice(b"\x1b[");
		for _ in 0..50 {
			seq.extend_from_slice(b"1;");
		}
		seq.push(b'H');
		let events = buf.process(&seq);
		let data = collect_data(&events);
		assert_eq!(data.len(), 1);
		assert_eq!(data[0].as_bytes(), seq.as_slice());
	}

	// ── Flush / Clear ───────────────────────────────────────────────

	#[test]
	fn flush_incomplete() {
		let mut buf = StdinBuffer::new();
		let _ = buf.process(b"\x1b[<35");
		let flushed = buf.flush();
		assert_eq!(flushed, vec!["\x1b[<35"]);
		assert!(buf.get_buffer().is_empty());
	}

	#[test]
	fn flush_empty() {
		let mut buf = StdinBuffer::new();
		let flushed = buf.flush();
		assert!(flushed.is_empty());
	}

	#[test]
	fn clear_buffer() {
		let mut buf = StdinBuffer::new();
		let _ = buf.process(b"\x1b[<35");
		assert!(!buf.get_buffer().is_empty());

		buf.clear();
		assert!(buf.get_buffer().is_empty());
	}

	// ── Bracketed Paste ─────────────────────────────────────────────

	#[test]
	fn complete_bracketed_paste() {
		let mut buf = StdinBuffer::new();
		let mut input = Vec::new();
		input.extend_from_slice(BRACKETED_PASTE_START);
		input.extend_from_slice(b"hello world");
		input.extend_from_slice(BRACKETED_PASTE_END);

		let events = buf.process(&input);
		assert_eq!(collect_paste(&events), vec!["hello world"]);
		assert!(collect_data(&events).is_empty());
	}

	#[test]
	fn paste_in_chunks() {
		let mut buf = StdinBuffer::new();
		let mut all = Vec::new();

		all.extend(buf.process(BRACKETED_PASTE_START));
		assert!(collect_paste(&all).is_empty());

		all.extend(buf.process(b"hello "));
		assert!(collect_paste(&all).is_empty());

		let mut end = b"world".to_vec();
		end.extend_from_slice(BRACKETED_PASTE_END);
		all.extend(buf.process(&end));
		assert_eq!(collect_paste(&all), vec!["hello world"]);
		assert!(collect_data(&all).is_empty());
	}

	#[test]
	fn paste_with_input_before_and_after() {
		let mut buf = StdinBuffer::new();
		let mut all_events = Vec::new();

		all_events.extend(buf.process(b"a"));

		let mut paste = Vec::new();
		paste.extend_from_slice(BRACKETED_PASTE_START);
		paste.extend_from_slice(b"pasted");
		paste.extend_from_slice(BRACKETED_PASTE_END);
		all_events.extend(buf.process(&paste));

		all_events.extend(buf.process(b"b"));

		assert_eq!(collect_data(&all_events), vec!["a", "b"]);
		assert_eq!(collect_paste(&all_events), vec!["pasted"]);
	}

	#[test]
	fn paste_with_newlines() {
		let mut buf = StdinBuffer::new();
		let mut input = Vec::new();
		input.extend_from_slice(BRACKETED_PASTE_START);
		input.extend_from_slice(b"line1\nline2\nline3");
		input.extend_from_slice(BRACKETED_PASTE_END);

		let events = buf.process(&input);
		assert_eq!(collect_paste(&events), vec!["line1\nline2\nline3"]);
	}

	#[test]
	fn paste_with_unicode() {
		let mut buf = StdinBuffer::new();
		let content = "Hello 世界 🎉";
		let mut input = Vec::new();
		input.extend_from_slice(BRACKETED_PASTE_START);
		input.extend_from_slice(content.as_bytes());
		input.extend_from_slice(BRACKETED_PASTE_END);

		let events = buf.process(&input);
		assert_eq!(collect_paste(&events), vec![content]);
	}

	// ── Destroy ─────────────────────────────────────────────────────

	#[test]
	fn clear_on_destroy() {
		let mut buf = StdinBuffer::new();
		let _ = buf.process(b"\x1b[<35");
		assert!(!buf.get_buffer().is_empty());

		buf.clear(); // destroy == clear in our impl
		assert!(buf.get_buffer().is_empty());
	}
}
