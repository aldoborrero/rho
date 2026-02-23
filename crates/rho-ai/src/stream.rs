use crate::{
	events::{AssistantMessageStream, StreamError},
	models::{Api, Model},
	provider::Provider,
	providers::{
		anthropic::AnthropicProvider, openai_completions::OpenAICompletionsProvider,
		openai_responses::OpenAIResponsesProvider,
	},
	types::{AssistantMessage, Context, StreamOptions},
};

fn resolve_provider(api: Api) -> Box<dyn Provider> {
	match api {
		Api::AnthropicMessages => Box::new(AnthropicProvider::new()),
		Api::OpenAICompletions => Box::new(OpenAICompletionsProvider::new()),
		Api::OpenAIResponses => Box::new(OpenAIResponsesProvider::new()),
	}
}

/// Start streaming from the appropriate provider based on model API type.
pub fn stream(model: &Model, context: &Context, options: &StreamOptions) -> AssistantMessageStream {
	let provider = resolve_provider(model.api);
	provider.stream(model, context, options)
}

/// Complete a request (stream + collect into a single `AssistantMessage`).
pub async fn complete(
	model: &Model,
	context: &Context,
	options: &StreamOptions,
) -> Result<AssistantMessage, StreamError> {
	stream(model, context, options).collect().await
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::models::*;

	#[test]
	fn resolve_provider_anthropic() {
		let provider = resolve_provider(Api::AnthropicMessages);
		assert_eq!(provider.name(), "anthropic");
	}

	#[test]
	fn resolve_provider_openai_completions() {
		let provider = resolve_provider(Api::OpenAICompletions);
		assert_eq!(provider.name(), "openai");
	}

	#[test]
	fn resolve_provider_openai_responses() {
		let provider = resolve_provider(Api::OpenAIResponses);
		assert_eq!(provider.name(), "openai-responses");
	}
}
