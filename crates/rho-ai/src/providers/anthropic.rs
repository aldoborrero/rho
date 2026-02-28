use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::{
	events::{AssistantMessageStream, AssistantMessageStreamWriter, StreamError, StreamEvent},
	models::Model,
	provider::Provider,
	providers::sse::parse_sse_stream,
	retry::is_retryable,
	types::*,
};

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

/// Provider implementation for the Anthropic Messages API.
pub struct AnthropicProvider {
	client: Client,
}

impl AnthropicProvider {
	pub fn new() -> Self {
		Self { client: Client::new() }
	}
}

impl Default for AnthropicProvider {
	fn default() -> Self {
		Self::new()
	}
}

#[async_trait]
impl Provider for AnthropicProvider {
	fn name(&self) -> &str {
		"anthropic"
	}

	fn stream(
		&self,
		model: &Model,
		context: &Context,
		options: &StreamOptions,
	) -> AssistantMessageStream {
		let (writer, stream) = AssistantMessageStream::new();
		let client = self.client.clone();
		let model = model.clone();
		let context = context.clone();
		let options = options.clone();

		tokio::spawn(async move {
			run_stream(client, &model, &context, &options, &writer).await;
		});

		stream
	}
}

// ---------------------------------------------------------------------------
// Internal: stream execution
// ---------------------------------------------------------------------------

async fn run_stream(
	client: Client,
	model: &Model,
	context: &Context,
	options: &StreamOptions,
	writer: &AssistantMessageStreamWriter,
) {
	// Resolve API key
	let api_key = match resolve_api_key(options) {
		Some(key) => key,
		None => {
			writer.error(
				StreamError {
					status:  None,
					message: "No API key found. Set ANTHROPIC_API_KEY or pass api_key in options."
						.into(),
				},
				false,
				None,
			);
			return;
		},
	};

	// Build request
	let url = format!("{}/v1/messages", model.base_url);
	let body = build_request_body(model, context, options);

	let mut builder = client
		.post(&url)
		.header("anthropic-version", "2023-06-01")
		.header("content-type", "application/json");

	// Auth header: OAuth vs API key
	if is_oauth_token(&api_key) {
		builder = builder.header("Authorization", format!("Bearer {}", api_key));
	} else {
		builder = builder.header("x-api-key", &api_key);
	}

	// Add custom headers
	for (key, value) in &options.headers {
		builder = builder.header(key, value);
	}

	// Send request
	let response = match builder.json(&body).send().await {
		Ok(resp) => resp,
		Err(err) => {
			let retryable = is_retryable(None, &err.to_string());
			writer.error(
				StreamError { status: None, message: format!("Request failed: {}", err) },
				retryable,
				None,
			);
			return;
		},
	};

	let status = response.status();
	if !status.is_success() {
		let status_code = status.as_u16();
		let body_text = response
			.text()
			.await
			.unwrap_or_else(|_| "<could not read body>".into());
		let retryable = is_retryable(Some(status_code), &body_text);
		let retry_after = crate::retry::parse_retry_after_from_error(&body_text);
		writer.error(
			StreamError {
				status:  Some(status_code),
				message: format!("Anthropic API returned {}: {}", status_code, body_text),
			},
			retryable,
			retry_after,
		);
		return;
	}

	// Process SSE stream
	let byte_stream = response.bytes_stream();
	let mut sse_stream = std::pin::pin!(parse_sse_stream(byte_stream));

	// State for accumulating the response
	let mut content_blocks: Vec<ContentBlock> = Vec::new();
	let mut block_types: Vec<BlockType> = Vec::new();
	let mut tool_json_accumulators: Vec<String> = Vec::new();
	let mut stop_reason: Option<StopReason> = None;
	let mut usage = Usage::default();

	while let Some(sse_event) = sse_stream.next().await {
		// Check cancellation
		if let Some(ref cancel) = options.abort {
			if cancel.is_cancelled() {
				writer.error(
					StreamError { status: None, message: "Request cancelled".into() },
					false,
					None,
				);
				return;
			}
		}

		let event_type = &sse_event.event;
		let data = match serde_json::from_str::<Value>(&sse_event.data) {
			Ok(v) => v,
			Err(_) => continue, // Skip unparseable data
		};

		match event_type.as_str() {
			"message_start" => {
				// Extract usage from message_start
				if let Some(msg_usage) = data.pointer("/message/usage") {
					if let Some(input) = msg_usage["input_tokens"].as_u64() {
						usage.input_tokens = input as u32;
					}
					if let Some(cache_read) = msg_usage["cache_read_input_tokens"].as_u64() {
						usage.cache_read_tokens = cache_read as u32;
					}
					if let Some(cache_write) = msg_usage["cache_creation_input_tokens"].as_u64() {
						usage.cache_write_tokens = cache_write as u32;
					}
				}
			},

			"content_block_start" => {
				let index = data["index"].as_u64().unwrap_or(0) as usize;
				let block = &data["content_block"];
				let block_type_str = block["type"].as_str().unwrap_or("");

				match block_type_str {
					"text" => {
						content_blocks.push(ContentBlock::Text { text: String::new() });
						block_types.push(BlockType::Text);
						tool_json_accumulators.push(String::new());
						writer.push(StreamEvent::TextStart { content_index: index });
					},
					"thinking" => {
						content_blocks.push(ContentBlock::Thinking { thinking: String::new() });
						block_types.push(BlockType::Thinking);
						tool_json_accumulators.push(String::new());
						writer.push(StreamEvent::ThinkingStart { content_index: index });
					},
					"tool_use" => {
						let id = block["id"].as_str().unwrap_or("").to_string();
						let name = block["name"].as_str().unwrap_or("").to_string();
						content_blocks.push(ContentBlock::ToolUse {
							id:    id.clone(),
							name:  name.clone(),
							input: Value::Null,
						});
						block_types.push(BlockType::ToolUse);
						tool_json_accumulators.push(String::new());
						writer.push(StreamEvent::ToolCallStart { content_index: index, id, name });
					},
					_ => {
						// Unknown block type, push placeholder
						content_blocks.push(ContentBlock::Text { text: String::new() });
						block_types.push(BlockType::Text);
						tool_json_accumulators.push(String::new());
					},
				}
			},

			"content_block_delta" => {
				let index = data["index"].as_u64().unwrap_or(0) as usize;
				let delta = &data["delta"];
				let delta_type = delta["type"].as_str().unwrap_or("");

				match delta_type {
					"text_delta" => {
						let text = delta["text"].as_str().unwrap_or("").to_string();
						if let Some(ContentBlock::Text { text: existing_text }) =
							content_blocks.get_mut(index)
						{
							existing_text.push_str(&text);
						}
						writer.push(StreamEvent::TextDelta { content_index: index, text });
					},
					"thinking_delta" => {
						let thinking = delta["thinking"].as_str().unwrap_or("").to_string();
						if let Some(ContentBlock::Thinking { thinking: existing_thinking }) =
							content_blocks.get_mut(index)
						{
							existing_thinking.push_str(&thinking);
						}
						writer.push(StreamEvent::ThinkingDelta { content_index: index, thinking });
					},
					"input_json_delta" => {
						let partial_json = delta["partial_json"].as_str().unwrap_or("").to_string();
						if let Some(acc) = tool_json_accumulators.get_mut(index) {
							acc.push_str(&partial_json);
						}
						writer.push(StreamEvent::ToolCallDelta { content_index: index, partial_json });
					},
					_ => {},
				}
			},

			"content_block_stop" => {
				let index = data["index"].as_u64().unwrap_or(0) as usize;

				// Finalize tool_use input JSON
				if let Some(ContentBlock::ToolUse { input, .. }) = content_blocks.get_mut(index) {
					if let Some(json_str) = tool_json_accumulators.get(index).filter(|s| !s.is_empty()) {
						if let Ok(parsed) = serde_json::from_str(json_str) {
							*input = parsed;
						}
					}
				}

				// Emit end event based on block type
				match block_types.get(index) {
					Some(BlockType::Text) => {
						writer.push(StreamEvent::TextEnd { content_index: index });
					},
					Some(BlockType::Thinking) => {
						writer.push(StreamEvent::ThinkingEnd { content_index: index });
					},
					Some(BlockType::ToolUse) => {
						writer.push(StreamEvent::ToolCallEnd { content_index: index });
					},
					None => {},
				}
			},

			"message_delta" => {
				if let Some(reason) = data["delta"]["stop_reason"].as_str() {
					stop_reason = Some(map_stop_reason(reason));
				}
				if let Some(delta_usage) = data.get("usage") {
					if let Some(output) = delta_usage["output_tokens"].as_u64() {
						usage.output_tokens = output as u32;
					}
				}
			},

			"message_stop" => {
				let message = AssistantMessage {
					content: content_blocks,
					stop_reason,
					usage: Some(usage.clone()),
				};
				writer.done(message, Some(usage));
				return;
			},

			"ping" => {
				// Ignore ping events
			},

			"error" => {
				let error_msg = data["error"]["message"]
					.as_str()
					.or_else(|| data["message"].as_str())
					.unwrap_or("Unknown error")
					.to_string();
				let retryable = is_retryable(None, &error_msg);
				writer.error(StreamError { status: None, message: error_msg }, retryable, None);
				return;
			},

			_ => {
				// Unknown event type, ignore
			},
		}
	}

	// Stream ended without message_stop - this shouldn't happen normally
	// but we handle it gracefully by assembling what we have
	let message =
		AssistantMessage { content: content_blocks, stop_reason, usage: Some(usage.clone()) };
	writer.done(message, Some(usage));
}

// ---------------------------------------------------------------------------
// Block type tracking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum BlockType {
	Text,
	Thinking,
	ToolUse,
}

// ---------------------------------------------------------------------------
// Helper: resolve API key
// ---------------------------------------------------------------------------

fn resolve_api_key(options: &StreamOptions) -> Option<String> {
	if let Some(ref key) = options.api_key {
		return Some(key.clone());
	}
	std::env::var("ANTHROPIC_API_KEY").ok()
}

// ---------------------------------------------------------------------------
// Public(crate) helpers
// ---------------------------------------------------------------------------

/// Check if a key is an OAuth token (vs. a traditional API key).
pub(crate) fn is_oauth_token(key: &str) -> bool {
	key.contains("sk-ant-oat")
}

/// Map an Anthropic stop reason string to our unified `StopReason`.
pub(crate) fn map_stop_reason(reason: &str) -> StopReason {
	match reason {
		"end_turn" => StopReason::Stop,
		"max_tokens" => StopReason::Length,
		"tool_use" => StopReason::ToolUse,
		"refusal" => StopReason::Error,
		_ => StopReason::Stop,
	}
}

/// Convert a single `Message` to the Anthropic API JSON format.
pub(crate) fn convert_message(msg: &Message) -> Value {
	match msg {
		Message::User(user) => {
			let content: Vec<Value> = user
				.content
				.iter()
				.map(|c| match c {
					UserContent::Text { text } => {
						serde_json::json!({"type": "text", "text": text})
					},
					UserContent::Image { data, mime_type } => {
						serde_json::json!({
							 "type": "image",
							 "source": {
								  "type": "base64",
								  "media_type": mime_type,
								  "data": data,
							 }
						})
					},
				})
				.collect();
			serde_json::json!({"role": "user", "content": content})
		},
		Message::Assistant(assistant) => {
			let content: Vec<Value> = assistant
				.content
				.iter()
				.filter_map(|block| match block {
					ContentBlock::Text { text } => {
						Some(serde_json::json!({"type": "text", "text": text}))
					},
					ContentBlock::Thinking { .. } => {
						// Thinking blocks are not sent back to the API.
						// Skip them entirely.
						None
					},
					ContentBlock::ToolUse { id, name, input } => Some(serde_json::json!({
						 "type": "tool_use",
						 "id": id,
						 "name": name,
						 "input": input,
					})),
				})
				.collect();
			serde_json::json!({"role": "assistant", "content": content})
		},
		Message::ToolResult(tool_result) => {
			let content: Vec<Value> = tool_result
				.content
				.iter()
				.map(|c| match c {
					ToolResultContent::Text { text } => {
						serde_json::json!({"type": "text", "text": text})
					},
					ToolResultContent::Image { data, mime_type } => {
						serde_json::json!({
							 "type": "image",
							 "source": {
								  "type": "base64",
								  "media_type": mime_type,
								  "data": data,
							 }
						})
					},
				})
				.collect();
			serde_json::json!({
				 "role": "user",
				 "content": [{
					  "type": "tool_result",
					  "tool_use_id": tool_result.tool_use_id,
					  "content": content,
					  "is_error": tool_result.is_error,
				 }]
			})
		},
	}
}

/// Convert all messages to the Anthropic API JSON format.
fn convert_messages(messages: &[Message]) -> Vec<Value> {
	messages.iter().map(convert_message).collect()
}

/// Build the thinking configuration for extended thinking.
pub(crate) fn build_thinking_config(options: &StreamOptions) -> Option<Value> {
	let level = options.reasoning.as_ref()?;

	let default_budgets = ThinkingBudgets {
		minimal: Some(1024),
		low:     Some(4096),
		medium:  Some(8192),
		high:    Some(16384),
		xhigh:   Some(32768),
	};

	let budgets = options
		.thinking_budgets
		.as_ref()
		.unwrap_or(&default_budgets);

	let budget = match level {
		ReasoningLevel::Minimal => budgets.minimal.unwrap_or(1024),
		ReasoningLevel::Low => budgets.low.unwrap_or(4096),
		ReasoningLevel::Medium => budgets.medium.unwrap_or(8192),
		ReasoningLevel::High => budgets.high.unwrap_or(16384),
		ReasoningLevel::XHigh => budgets.xhigh.unwrap_or(32768),
	};

	Some(serde_json::json!({
		 "type": "enabled",
		 "budget_tokens": budget,
	}))
}

/// Build cache control for a content block.
fn build_cache_control(retention: &CacheRetention) -> Option<Value> {
	match retention {
		CacheRetention::None => None,
		CacheRetention::Short => Some(serde_json::json!({"type": "ephemeral"})),
		CacheRetention::Long => Some(serde_json::json!({"type": "ephemeral"})),
	}
}

/// Build the full Anthropic Messages API request body.
pub(crate) fn build_request_body(
	model: &Model,
	context: &Context,
	options: &StreamOptions,
) -> Value {
	let max_tokens = options.max_tokens.unwrap_or(model.max_tokens);
	let messages = convert_messages(&context.messages);

	let mut body = serde_json::json!({
		 "model": model.id,
		 "max_tokens": max_tokens,
		 "stream": true,
		 "messages": messages,
	});

	// System prompt
	if let Some(ref system_prompt) = context.system_prompt {
		let mut system_block = serde_json::json!({"type": "text", "text": system_prompt});
		if let Some(cache) = build_cache_control(&options.cache_retention) {
			system_block["cache_control"] = cache;
		}
		body["system"] = serde_json::json!([system_block]);
	}

	// Tools
	if !context.tools.is_empty() {
		let tools: Vec<Value> = context
			.tools
			.iter()
			.map(|t| {
				serde_json::json!({
					 "name": t.name,
					 "description": t.description,
					 "input_schema": t.input_schema,
				})
			})
			.collect();
		body["tools"] = Value::Array(tools);
	}

	// Tool choice
	if let Some(tc) = crate::tool_choice::to_anthropic(options.tool_choice.as_ref()) {
		body["tool_choice"] = tc;
	}

	// Temperature
	if let Some(temp) = options.temperature {
		body["temperature"] = serde_json::json!(temp);
	}

	// Thinking
	if let Some(thinking) = build_thinking_config(options) {
		body["thinking"] = thinking;
	}

	body
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use super::*;

	#[test]
	fn is_oauth_token_detection() {
		assert!(is_oauth_token("sk-ant-oat-abc123"));
		assert!(!is_oauth_token("sk-ant-api01-abc123"));
		assert!(!is_oauth_token("regular-key"));
	}

	#[test]
	fn build_request_basic() {
		let model = test_model();
		let ctx = Context {
			system_prompt: Some(Arc::new("You are helpful.".into())),
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["model"], "claude-sonnet-4-5-20250929");
		assert_eq!(body["stream"], true);
		assert!(body["system"].is_array());
	}

	#[test]
	fn build_request_with_tools() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "run ls".into() }],
			})],
			tools:         vec![ToolDefinition {
				name:         "bash".into(),
				description:  "Run a command".into(),
				input_schema: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
			}],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert!(body["tools"].is_array());
		assert_eq!(body["tools"].as_array().unwrap().len(), 1);
		assert_eq!(body["tools"][0]["name"], "bash");
	}

	#[test]
	fn build_request_with_temperature() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions { temperature: Some(0.5), ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["temperature"], 0.5);
	}

	#[test]
	fn build_request_max_tokens_override() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions { max_tokens: Some(4096), ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["max_tokens"], 4096);
	}

	#[test]
	fn build_request_uses_model_max_tokens_by_default() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["max_tokens"], 8192);
	}

	#[test]
	fn convert_user_message_text() {
		let msg =
			Message::User(UserMessage { content: vec![UserContent::Text { text: "hello".into() }] });
		let api = convert_message(&msg);
		assert_eq!(api["role"], "user");
		let content = api["content"].as_array().unwrap();
		assert_eq!(content.len(), 1);
		assert_eq!(content[0]["type"], "text");
		assert_eq!(content[0]["text"], "hello");
	}

	#[test]
	fn convert_user_message_image() {
		let msg = Message::User(UserMessage {
			content: vec![UserContent::Image {
				data:      "base64data".into(),
				mime_type: "image/png".into(),
			}],
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "user");
		let content = api["content"].as_array().unwrap();
		assert_eq!(content[0]["type"], "image");
		assert_eq!(content[0]["source"]["type"], "base64");
		assert_eq!(content[0]["source"]["media_type"], "image/png");
		assert_eq!(content[0]["source"]["data"], "base64data");
	}

	#[test]
	fn convert_assistant_with_tool_use() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me check.".into() },
				ContentBlock::ToolUse {
					id:    "tc_1".into(),
					name:  "read_file".into(),
					input: serde_json::json!({"path": "/tmp"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "assistant");
		let content = api["content"].as_array().unwrap();
		assert_eq!(content.len(), 2);
		assert_eq!(content[0]["type"], "text");
		assert_eq!(content[0]["text"], "Let me check.");
		assert_eq!(content[1]["type"], "tool_use");
		assert_eq!(content[1]["id"], "tc_1");
		assert_eq!(content[1]["name"], "read_file");
	}

	#[test]
	fn convert_assistant_thinking_is_skipped() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Thinking { thinking: "internal reasoning".into() },
				ContentBlock::Text { text: "visible response".into() },
			],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		});
		let api = convert_message(&msg);
		let content = api["content"].as_array().unwrap();
		// Thinking block should be skipped entirely
		assert_eq!(content.len(), 1);
		assert_eq!(content[0]["type"], "text");
		assert_eq!(content[0]["text"], "visible response");
	}

	#[test]
	fn convert_tool_result() {
		let msg = Message::ToolResult(ToolResultMessage {
			tool_use_id: "tc_1".into(),
			content:     vec![ToolResultContent::Text { text: "file contents".into() }],
			is_error:    false,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "user");
		let content = api["content"].as_array().unwrap();
		assert_eq!(content[0]["type"], "tool_result");
		assert_eq!(content[0]["tool_use_id"], "tc_1");
		assert!(!content[0]["is_error"].as_bool().unwrap());
		let inner_content = content[0]["content"].as_array().unwrap();
		assert_eq!(inner_content[0]["type"], "text");
		assert_eq!(inner_content[0]["text"], "file contents");
	}

	#[test]
	fn convert_tool_result_with_error() {
		let msg = Message::ToolResult(ToolResultMessage {
			tool_use_id: "tc_2".into(),
			content:     vec![ToolResultContent::Text { text: "command not found".into() }],
			is_error:    true,
		});
		let api = convert_message(&msg);
		let content = api["content"].as_array().unwrap();
		assert!(content[0]["is_error"].as_bool().unwrap());
	}

	#[test]
	fn thinking_config_budget() {
		let options = StreamOptions { reasoning: Some(ReasoningLevel::Medium), ..Default::default() };
		let thinking = build_thinking_config(&options);
		assert!(thinking.is_some());
		let t = thinking.unwrap();
		assert_eq!(t["type"], "enabled");
		assert_eq!(t["budget_tokens"].as_u64().unwrap(), 8192);
	}

	#[test]
	fn thinking_config_all_levels() {
		for (level, expected_budget) in [
			(ReasoningLevel::Minimal, 1024),
			(ReasoningLevel::Low, 4096),
			(ReasoningLevel::Medium, 8192),
			(ReasoningLevel::High, 16384),
			(ReasoningLevel::XHigh, 32768),
		] {
			let options = StreamOptions { reasoning: Some(level), ..Default::default() };
			let thinking = build_thinking_config(&options).unwrap();
			assert_eq!(thinking["budget_tokens"].as_u64().unwrap(), expected_budget);
		}
	}

	#[test]
	fn thinking_config_none_when_no_reasoning() {
		let options = StreamOptions::default();
		assert!(build_thinking_config(&options).is_none());
	}

	#[test]
	fn thinking_config_custom_budgets() {
		let options = StreamOptions {
			reasoning: Some(ReasoningLevel::Medium),
			thinking_budgets: Some(ThinkingBudgets {
				minimal: Some(512),
				low:     Some(2048),
				medium:  Some(4096),
				high:    Some(8192),
				xhigh:   Some(16384),
			}),
			..Default::default()
		};
		let thinking = build_thinking_config(&options).unwrap();
		assert_eq!(thinking["budget_tokens"].as_u64().unwrap(), 4096);
	}

	#[test]
	fn stop_reason_mapping() {
		assert_eq!(map_stop_reason("end_turn"), StopReason::Stop);
		assert_eq!(map_stop_reason("max_tokens"), StopReason::Length);
		assert_eq!(map_stop_reason("tool_use"), StopReason::ToolUse);
		assert_eq!(map_stop_reason("refusal"), StopReason::Error);
	}

	#[test]
	fn stop_reason_unknown_defaults_to_stop() {
		assert_eq!(map_stop_reason("unknown_reason"), StopReason::Stop);
		assert_eq!(map_stop_reason(""), StopReason::Stop);
	}

	#[test]
	fn cache_control_none() {
		assert!(build_cache_control(&CacheRetention::None).is_none());
	}

	#[test]
	fn cache_control_short() {
		let cc = build_cache_control(&CacheRetention::Short).unwrap();
		assert_eq!(cc["type"], "ephemeral");
	}

	#[test]
	fn cache_control_long() {
		let cc = build_cache_control(&CacheRetention::Long).unwrap();
		assert_eq!(cc["type"], "ephemeral");
	}

	#[test]
	fn system_prompt_with_cache_control() {
		let model = test_model();
		let ctx = Context {
			system_prompt: Some(Arc::new("Be helpful.".into())),
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions { cache_retention: CacheRetention::Short, ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		let system = body["system"].as_array().unwrap();
		assert_eq!(system.len(), 1);
		assert_eq!(system[0]["text"], "Be helpful.");
		assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
	}

	#[test]
	fn system_prompt_without_cache_control() {
		let model = test_model();
		let ctx = Context {
			system_prompt: Some(Arc::new("Be helpful.".into())),
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions { cache_retention: CacheRetention::None, ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		let system = body["system"].as_array().unwrap();
		assert_eq!(system.len(), 1);
		assert!(system[0].get("cache_control").is_none());
	}

	#[test]
	fn no_system_prompt() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert!(body.get("system").is_none());
	}

	#[test]
	fn provider_name() {
		let provider = AnthropicProvider::new();
		assert_eq!(provider.name(), "anthropic");
	}

	fn test_model() -> crate::models::Model {
		crate::models::Model {
			id:              "claude-sonnet-4-5-20250929".into(),
			name:            "Claude Sonnet 4.5".into(),
			provider:        "anthropic".into(),
			api:             crate::models::Api::AnthropicMessages,
			base_url:        "https://api.anthropic.com".into(),
			reasoning:       false,
			supports_images: true,
			context_window:  200000,
			max_tokens:      8192,
			cost:            Default::default(),
		}
	}
}
