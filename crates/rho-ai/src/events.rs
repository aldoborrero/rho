use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::types::{AssistantMessage, Usage};

// ---------------------------------------------------------------------------
// StreamError
// ---------------------------------------------------------------------------

/// An error received during streaming.
#[derive(Debug, Clone)]
pub struct StreamError {
	pub status:  Option<u16>,
	pub message: String,
}

impl std::fmt::Display for StreamError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if let Some(status) = self.status {
			write!(f, "[{}] {}", status, self.message)
		} else {
			write!(f, "{}", self.message)
		}
	}
}

impl std::error::Error for StreamError {}

// ---------------------------------------------------------------------------
// StreamEvent
// ---------------------------------------------------------------------------

/// Events emitted during an LLM streaming response.
#[derive(Debug, Clone)]
pub enum StreamEvent {
	TextStart { content_index: usize },
	TextDelta { content_index: usize, text: String },
	TextEnd { content_index: usize },
	ThinkingStart { content_index: usize },
	ThinkingDelta { content_index: usize, thinking: String },
	ThinkingEnd { content_index: usize },
	ToolCallStart { content_index: usize, id: String, name: String },
	ToolCallDelta { content_index: usize, partial_json: String },
	ToolCallEnd { content_index: usize },
	Done { message: AssistantMessage, usage: Option<Usage> },
	Error { error: StreamError, retryable: bool, retry_after_ms: Option<u64> },
}

// ---------------------------------------------------------------------------
// AssistantMessageStreamWriter
// ---------------------------------------------------------------------------

/// The write half of a streaming assistant response.
///
/// Providers push [`StreamEvent`]s into this writer; consumers read from the
/// paired [`AssistantMessageStream`].
pub struct AssistantMessageStreamWriter {
	tx: mpsc::UnboundedSender<StreamEvent>,
}

impl AssistantMessageStreamWriter {
	/// Push a single event into the stream.
	pub fn push(&self, event: StreamEvent) {
		let _ = self.tx.send(event);
	}

	/// Signal that the stream completed successfully.
	pub fn done(&self, message: AssistantMessage, usage: Option<Usage>) {
		let _ = self.tx.send(StreamEvent::Done { message, usage });
	}

	/// Signal that the stream encountered an error.
	pub fn error(&self, error: StreamError, retryable: bool, retry_after_ms: Option<u64>) {
		let _ = self
			.tx
			.send(StreamEvent::Error { error, retryable, retry_after_ms });
	}
}

// ---------------------------------------------------------------------------
// AssistantMessageStream
// ---------------------------------------------------------------------------

/// The read half of a streaming assistant response.
///
/// Created via [`AssistantMessageStream::new`], which returns both the stream
/// and its paired [`AssistantMessageStreamWriter`].
pub struct AssistantMessageStream {
	rx: mpsc::UnboundedReceiver<StreamEvent>,
}

impl AssistantMessageStream {
	/// Create a new `(writer, stream)` pair.
	pub fn new() -> (AssistantMessageStreamWriter, Self) {
		let (tx, rx) = mpsc::unbounded_channel();
		(AssistantMessageStreamWriter { tx }, Self { rx })
	}

	/// Drain all events and return the final [`AssistantMessage`].
	///
	/// Returns `Ok(message)` when a `Done` event is received, `Err` when an
	/// `Error` event is received or the channel closes without a `Done`.
	pub async fn collect(mut self) -> Result<AssistantMessage, StreamError> {
		while let Some(event) = self.rx.recv().await {
			match event {
				StreamEvent::Done { message, .. } => return Ok(message),
				StreamEvent::Error { error, .. } => return Err(error),
				_ => { /* consume intermediate events */ },
			}
		}
		// Channel closed without Done
		Err(StreamError { status: None, message: "stream ended without a Done event".into() })
	}

	/// Convert into a [`tokio_stream::Stream`] of [`StreamEvent`]s.
	pub fn into_stream(self) -> UnboundedReceiverStream<StreamEvent> {
		UnboundedReceiverStream::new(self.rx)
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::*;

	#[tokio::test]
	async fn stream_text_events() {
		let (writer, stream) = AssistantMessageStream::new();
		writer.push(StreamEvent::TextStart { content_index: 0 });
		writer.push(StreamEvent::TextDelta { content_index: 0, text: "Hello".into() });
		writer.push(StreamEvent::TextDelta { content_index: 0, text: " world".into() });
		writer.push(StreamEvent::TextEnd { content_index: 0 });
		let msg = AssistantMessage {
			content:     vec![ContentBlock::Text { text: "Hello world".into() }],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		};
		writer.done(msg.clone(), None);
		drop(writer);

		let result = stream.collect().await.unwrap();
		assert_eq!(result.content.len(), 1);
		assert!(matches!(&result.content[0], ContentBlock::Text { text } if text == "Hello world"));
	}

	#[tokio::test]
	async fn stream_collects_done_message() {
		let (writer, stream) = AssistantMessageStream::new();
		let msg = AssistantMessage {
			content:     vec![ContentBlock::Text { text: "hi".into() }],
			stop_reason: Some(StopReason::Stop),
			usage:       Some(Usage { input_tokens: 10, output_tokens: 5, ..Default::default() }),
		};
		writer.done(msg, None);
		drop(writer);

		let result = stream.collect().await.unwrap();
		assert_eq!(result.usage.unwrap().input_tokens, 10);
	}

	#[tokio::test]
	async fn stream_error_returns_err() {
		let (writer, stream) = AssistantMessageStream::new();
		writer.error(StreamError { status: Some(500), message: "server error".into() }, true, None);
		drop(writer);

		let result = stream.collect().await;
		assert!(result.is_err());
	}

	#[tokio::test]
	async fn stream_as_async_iterator() {
		use tokio_stream::StreamExt;

		let (writer, stream) = AssistantMessageStream::new();
		writer.push(StreamEvent::TextDelta { content_index: 0, text: "hi".into() });
		let msg = AssistantMessage {
			content:     vec![ContentBlock::Text { text: "hi".into() }],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		};
		writer.done(msg, None);
		drop(writer);

		let events: Vec<StreamEvent> = stream.into_stream().collect().await;
		assert!(events.len() >= 2); // at least delta + done
	}

	#[tokio::test]
	async fn writer_drop_ends_stream() {
		let (writer, stream) = AssistantMessageStream::new();
		drop(writer);
		let result = stream.collect().await;
		assert!(result.is_err()); // no Done event = error
	}
}
