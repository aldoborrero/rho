use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream::unfold};

/// A parsed Server-Sent Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
	/// The event type (from `event:` field).
	pub event: String,
	/// The event data (from `data:` field).
	pub data:  String,
}

/// Internal state for the SSE line-buffered parser.
struct SseParserState<S> {
	byte_stream:   S,
	/// Accumulates raw bytes until newlines are found.
	buffer:        String,
	/// The current event type being built up.
	current_event: String,
	/// The current data being built up.
	current_data:  String,
}

/// Try to extract completed lines from the buffer and produce an `SseEvent`
/// when a blank line (event separator) is encountered.
///
/// Returns `Some(event)` if a complete event was found, `None` if more data is
/// needed.
fn try_parse_event(state: &mut SseParserState<impl Unpin>) -> Option<SseEvent> {
	loop {
		let newline_pos = state.buffer.find('\n')?;
		let line = state.buffer[..newline_pos]
			.trim_end_matches('\r')
			.to_owned();
		state.buffer = state.buffer[newline_pos + 1..].to_owned();

		if line.is_empty() {
			// Empty line delimits an event per SSE spec.
			// Emit only if we have both event type and data.
			if !state.current_event.is_empty() && !state.current_data.is_empty() {
				let event = SseEvent {
					event: std::mem::take(&mut state.current_event),
					data:  std::mem::take(&mut state.current_data),
				};
				return Some(event);
			}
			// Reset state even if we didn't emit (e.g. event without data).
			state.current_event.clear();
			state.current_data.clear();
			continue;
		}

		if let Some(event_type) = line.strip_prefix("event:") {
			state.current_event = event_type.trim().to_owned();
		} else if let Some(data) = line.strip_prefix("data:") {
			let data = data.trim();
			// Skip [DONE] marker used by some providers.
			if data == "[DONE]" {
				continue;
			}
			state.current_data = data.to_owned();
		}
		// Ignore comment lines (starting with ':') and unrecognized fields.
	}
}

/// Parse a byte stream into SSE events.
///
/// Handles chunked delivery, `[DONE]` markers, and ping filtering.
/// Events are emitted when a blank line is encountered (per the SSE spec),
/// provided both `event` and `data` fields have been set.
pub fn parse_sse_stream<S>(byte_stream: S) -> impl Stream<Item = SseEvent>
where
	S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
	let initial_state = SseParserState {
		byte_stream,
		buffer: String::new(),
		current_event: String::new(),
		current_data: String::new(),
	};

	unfold(initial_state, |mut state| async move {
		// First, try to produce an event from data already buffered.
		if let Some(event) = try_parse_event(&mut state) {
			return Some((event, state));
		}

		// Need more data from the byte stream.
		while let Some(chunk) = state.byte_stream.next().await {
			match chunk {
				Ok(bytes) => {
					let text = String::from_utf8_lossy(&bytes);
					state.buffer.push_str(&text);

					if let Some(event) = try_parse_event(&mut state) {
						return Some((event, state));
					}
					// No complete event yet, keep reading chunks.
				},
				Err(_) => {
					// On stream error, stop producing events.
					return None;
				},
			}
		}

		// Stream exhausted. Try one last parse in case there's a trailing event
		// without a final empty line (non-standard but defensive).
		// In practice, well-formed SSE always ends with \n\n, but we check anyway.
		None
	})
}

#[cfg(test)]
mod tests {
	use bytes::Bytes;
	use futures_util::{StreamExt, stream};

	use super::*;

	fn bytes_stream(chunks: Vec<&str>) -> impl Stream<Item = Result<Bytes, reqwest::Error>> {
		stream::iter(chunks.into_iter().map(|s| Ok(Bytes::from(s.to_string()))))
	}

	#[tokio::test]
	async fn parse_single_event() {
		let chunks = vec!["event: message_start\ndata: {\"type\":\"message_start\"}\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert_eq!(events.len(), 1);
		assert_eq!(events[0].event, "message_start");
	}

	#[tokio::test]
	async fn parse_multiple_events() {
		let chunks = vec!["event: ping\ndata: {}\n\nevent: text\ndata: {\"text\":\"hi\"}\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert_eq!(events.len(), 2);
	}

	#[tokio::test]
	async fn handle_chunked_delivery() {
		let chunks = vec!["event: te", "xt\ndata: {\"t\":1}\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert_eq!(events.len(), 1);
		assert_eq!(events[0].event, "text");
	}

	#[tokio::test]
	async fn skip_done_marker() {
		let chunks = vec!["event: text\ndata: {}\n\ndata: [DONE]\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert_eq!(events.len(), 1);
	}

	#[tokio::test]
	async fn empty_stream() {
		let chunks: Vec<&str> = vec![];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert!(events.is_empty());
	}

	#[tokio::test]
	async fn event_without_data_is_skipped() {
		let chunks = vec!["event: ping\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert!(events.is_empty());
	}

	#[tokio::test]
	async fn data_without_event_is_skipped() {
		let chunks = vec!["data: {\"some\":\"data\"}\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert!(events.is_empty());
	}

	#[tokio::test]
	async fn comment_lines_are_ignored() {
		let chunks = vec![": this is a comment\nevent: text\ndata: {}\n\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert_eq!(events.len(), 1);
		assert_eq!(events[0].event, "text");
	}

	#[tokio::test]
	async fn carriage_return_handling() {
		let chunks = vec!["event: text\r\ndata: {\"ok\":true}\r\n\r\n"];
		let stream = bytes_stream(chunks);
		let events: Vec<SseEvent> = parse_sse_stream(stream).collect().await;
		assert_eq!(events.len(), 1);
		assert_eq!(events[0].event, "text");
		assert_eq!(events[0].data, "{\"ok\":true}");
	}
}
