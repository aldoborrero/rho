use async_trait::async_trait;

use crate::{
	events::AssistantMessageStream,
	models::Model,
	types::{Context, StreamOptions},
};

/// Trait implemented by all LLM providers (Anthropic, OpenAI Completions,
/// OpenAI Responses).
///
/// Each provider knows how to turn a [`Context`] + [`StreamOptions`] into a
/// streaming response by spawning a background tokio task that pushes
/// [`StreamEvent`]s into the returned [`AssistantMessageStream`].
#[async_trait]
pub trait Provider: Send + Sync {
	/// Unique provider identifier (e.g., "anthropic", "openai").
	fn name(&self) -> &str;

	/// Start streaming a response. Spawns a background task that writes
	/// events to the stream. Returns immediately.
	fn stream(
		&self,
		model: &Model,
		context: &Context,
		options: &StreamOptions,
	) -> AssistantMessageStream;
}
