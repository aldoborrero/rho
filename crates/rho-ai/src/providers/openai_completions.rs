use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::{
	events::{AssistantMessageStream, AssistantMessageStreamWriter, StreamError, StreamEvent},
	models::Model,
	provider::Provider,
	retry::is_retryable,
	types::*,
};

// ---------------------------------------------------------------------------
// OpenAICompletionsProvider
// ---------------------------------------------------------------------------

/// Provider implementation for the OpenAI Chat Completions API.
pub struct OpenAICompletionsProvider {
	client: Client,
}

impl OpenAICompletionsProvider {
	pub fn new() -> Self {
		Self { client: Client::new() }
	}
}

impl Default for OpenAICompletionsProvider {
	fn default() -> Self {
		Self::new()
	}
}

#[async_trait]
impl Provider for OpenAICompletionsProvider {
	fn name(&self) -> &str {
		"openai"
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
					message: "No API key found. Set OPENAI_API_KEY or pass api_key in options.".into(),
				},
				false,
				None,
			);
			return;
		},
	};

	// Build request
	let url = format!("{}/v1/chat/completions", model.base_url);
	let body = build_request_body(model, context, options);

	let mut builder = client
		.post(&url)
		.header("Authorization", format!("Bearer {}", api_key))
		.header("content-type", "application/json");

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
				message: format!("OpenAI API returned {}: {}", status_code, body_text),
			},
			retryable,
			retry_after,
		);
		return;
	}

	// Process SSE stream
	// OpenAI sends SSE without `event:` lines, just `data: {...}` followed by blank
	// lines. We parse lines manually instead of using the shared SSE parser.
	let byte_stream = response.bytes_stream();
	let mut byte_stream = std::pin::pin!(byte_stream);

	// State for accumulating the response
	let mut content_blocks: Vec<ContentBlock> = Vec::new();
	let mut stop_reason: Option<StopReason> = None;
	let mut usage = Usage::default();

	// Tool call assembly state: track by index in delta.tool_calls
	// Maps tool_call index -> (id, name, accumulated_arguments)
	let mut tool_calls: Vec<ToolCallState> = Vec::new();

	// Content index counter for stream events
	let mut next_content_index: usize = 0;
	// Whether we've started a text block
	let mut text_started = false;
	let mut accumulated_text = String::new();

	// Line buffer for SSE parsing
	let mut line_buffer = String::new();

	while let Some(chunk) = byte_stream.next().await {
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

		let bytes = match chunk {
			Ok(b) => b,
			Err(err) => {
				let retryable = is_retryable(None, &err.to_string());
				writer.error(
					StreamError { status: None, message: format!("Stream read error: {}", err) },
					retryable,
					None,
				);
				return;
			},
		};

		let text = String::from_utf8_lossy(&bytes);
		line_buffer.push_str(&text);

		// Process complete lines
		while let Some(newline_pos) = line_buffer.find('\n') {
			let line = line_buffer[..newline_pos].trim_end_matches('\r').to_owned();
			line_buffer = line_buffer[newline_pos + 1..].to_owned();

			// Skip empty lines and comments
			if line.is_empty() || line.starts_with(':') {
				continue;
			}

			// Parse data: lines
			let data_str = if let Some(d) = line.strip_prefix("data:") {
				d.trim()
			} else {
				continue;
			};

			// Skip [DONE] marker
			if data_str == "[DONE]" {
				continue;
			}

			// Parse the JSON
			let data: Value = match serde_json::from_str(data_str) {
				Ok(v) => v,
				Err(_) => continue,
			};

			// Process the chunk
			// OpenAI format:
			// {"id":"...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{.
			// ..},"finish_reason":null}],"usage":{...}}

			if let Some(choices) = data["choices"].as_array() {
				for choice in choices {
					let delta = &choice["delta"];

					// Handle content text delta
					if let Some(content_text) = delta["content"].as_str() {
						if !text_started {
							text_started = true;
							let idx = next_content_index;
							writer.push(StreamEvent::TextStart { content_index: idx });
						}
						accumulated_text.push_str(content_text);
						writer.push(StreamEvent::TextDelta {
							content_index: next_content_index,
							text:          content_text.to_string(),
						});
					}

					// Handle tool_calls delta
					if let Some(tc_deltas) = delta["tool_calls"].as_array() {
						// If we had text going, close it first
						if text_started {
							content_blocks
								.push(ContentBlock::Text { text: std::mem::take(&mut accumulated_text) });
							writer.push(StreamEvent::TextEnd { content_index: next_content_index });
							next_content_index += 1;
							text_started = false;
						}

						for tc_delta in tc_deltas {
							let tc_index = tc_delta["index"].as_u64().unwrap_or(0) as usize;

							// Ensure tool_calls vec is large enough
							while tool_calls.len() <= tc_index {
								tool_calls.push(ToolCallState::default());
							}

							let tc_state = &mut tool_calls[tc_index];

							// First chunk has id and function.name
							if let Some(id) = tc_delta["id"].as_str() {
								tc_state.id = id.to_string();
							}
							if let Some(name) = tc_delta["function"]["name"].as_str() {
								tc_state.name = name.to_string();
								// Assign content index when we first see the tool call
								tc_state.content_index = next_content_index + tc_index;

								writer.push(StreamEvent::ToolCallStart {
									content_index: tc_state.content_index,
									id:            tc_state.id.clone(),
									name:          tc_state.name.clone(),
								});
							}
							if let Some(args) = tc_delta["function"]["arguments"].as_str() {
								tc_state.arguments.push_str(args);
								writer.push(StreamEvent::ToolCallDelta {
									content_index: tc_state.content_index,
									partial_json:  args.to_string(),
								});
							}
						}
					}

					// Handle finish_reason
					if let Some(reason) = choice["finish_reason"].as_str() {
						stop_reason = Some(map_finish_reason(Some(reason)));
					}
				}
			}

			// Handle usage (top-level, on final chunk with stream_options.include_usage)
			if let Some(usage_obj) = data.get("usage") {
				if !usage_obj.is_null() {
					if let Some(input) = usage_obj["prompt_tokens"].as_u64() {
						usage.input_tokens = input as u32;
					}
					if let Some(output) = usage_obj["completion_tokens"].as_u64() {
						usage.output_tokens = output as u32;
					}
					// OpenAI doesn't have cache tokens in the standard API, but some
					// providers (Azure, etc.) may include them
					if let Some(cache_read) =
						usage_obj["prompt_tokens_details"]["cached_tokens"].as_u64()
					{
						usage.cache_read_tokens = cache_read as u32;
					}
				}
			}
		}
	}

	// Finalize: close any open text block
	if text_started {
		content_blocks.push(ContentBlock::Text { text: accumulated_text });
		writer.push(StreamEvent::TextEnd { content_index: next_content_index });
	}

	// Finalize tool calls
	for tc_state in &tool_calls {
		let input = if tc_state.arguments.is_empty() {
			Value::Null
		} else {
			serde_json::from_str(&tc_state.arguments).unwrap_or(Value::Null)
		};
		content_blocks.push(ContentBlock::ToolUse {
			id: tc_state.id.clone(),
			name: tc_state.name.clone(),
			input,
		});
		writer.push(StreamEvent::ToolCallEnd { content_index: tc_state.content_index });
	}

	let message =
		AssistantMessage { content: content_blocks, stop_reason, usage: Some(usage.clone()) };
	writer.done(message, Some(usage));
}

// ---------------------------------------------------------------------------
// Tool call assembly state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct ToolCallState {
	id:            String,
	name:          String,
	arguments:     String,
	content_index: usize,
}

// ---------------------------------------------------------------------------
// Helper: resolve API key
// ---------------------------------------------------------------------------

fn resolve_api_key(options: &StreamOptions) -> Option<String> {
	if let Some(ref key) = options.api_key {
		return Some(key.clone());
	}
	std::env::var("OPENAI_API_KEY").ok()
}

// ---------------------------------------------------------------------------
// Public(crate) helpers
// ---------------------------------------------------------------------------

/// Map an OpenAI finish_reason string to our unified `StopReason`.
pub(crate) fn map_finish_reason(reason: Option<&str>) -> StopReason {
	match reason {
		Some("stop") => StopReason::Stop,
		Some("length") => StopReason::Length,
		Some("tool_calls") => StopReason::ToolUse,
		Some("content_filter") => StopReason::Error,
		_ => StopReason::Stop,
	}
}

/// Convert a single `Message` to the OpenAI Chat Completions API JSON format.
pub(crate) fn convert_message(msg: &Message) -> Value {
	match msg {
		Message::User(user) => {
			// Single text content: use simple string format
			// Multi-part or image: use array format
			if user.content.len() == 1 {
				if let UserContent::Text { text } = &user.content[0] {
					return serde_json::json!({"role": "user", "content": text});
				}
			}

			let content: Vec<Value> = user
				.content
				.iter()
				.map(|c| match c {
					UserContent::Text { text } => {
						serde_json::json!({"type": "text", "text": text})
					},
					UserContent::Image { data, mime_type } => {
						serde_json::json!({
							 "type": "image_url",
							 "image_url": {
								  "url": format!("data:{};base64,{}", mime_type, data),
							 }
						})
					},
				})
				.collect();
			serde_json::json!({"role": "user", "content": content})
		},
		Message::Assistant(assistant) => {
			let mut text_parts: Vec<String> = Vec::new();
			let mut tool_calls: Vec<Value> = Vec::new();

			for block in &assistant.content {
				match block {
					ContentBlock::Text { text } => {
						text_parts.push(text.clone());
					},
					ContentBlock::Thinking { thinking } => {
						// OpenAI doesn't have native thinking support.
						// Prepend thinking as text to preserve context.
						text_parts.push(thinking.clone());
					},
					ContentBlock::ToolUse { id, name, input } => {
						tool_calls.push(serde_json::json!({
							 "id": id,
							 "type": "function",
							 "function": {
								  "name": name,
								  "arguments": input.to_string(),
							 }
						}));
					},
				}
			}

			let content_text = text_parts.join("\n");

			let mut result = serde_json::json!({"role": "assistant"});
			if tool_calls.is_empty() {
				result["content"] = Value::String(content_text);
			} else {
				if content_text.is_empty() {
					result["content"] = Value::Null;
				} else {
					result["content"] = Value::String(content_text);
				}
				result["tool_calls"] = Value::Array(tool_calls);
			}

			result
		},
		Message::ToolResult(tool_result) => {
			// OpenAI tool results: { "role": "tool", "tool_call_id": "...", "content":
			// "..." }
			let content_text: String = tool_result
				.content
				.iter()
				.filter_map(|c| match c {
					ToolResultContent::Text { text } => Some(text.as_str()),
					ToolResultContent::Image { .. } => None, // OpenAI tool results don't support images
				})
				.collect::<Vec<&str>>()
				.join("\n");

			serde_json::json!({
				 "role": "tool",
				 "tool_call_id": tool_result.tool_use_id,
				 "content": content_text,
			})
		},
	}
}

/// Convert all messages to the OpenAI Chat Completions API JSON format.
fn convert_messages(messages: &[Message]) -> Vec<Value> {
	messages.iter().map(convert_message).collect()
}

/// Build the full OpenAI Chat Completions API request body.
pub(crate) fn build_request_body(
	model: &Model,
	context: &Context,
	options: &StreamOptions,
) -> Value {
	let max_tokens = options.max_tokens.unwrap_or(model.max_tokens);
	let mut messages = Vec::new();

	// System prompt as first message
	if let Some(ref system_prompt) = context.system_prompt {
		messages.push(serde_json::json!({
			 "role": "system",
			 "content": system_prompt,
		}));
	}

	// Conversation messages
	messages.extend(convert_messages(&context.messages));

	let mut body = serde_json::json!({
		 "model": model.id,
		 "max_tokens": max_tokens,
		 "stream": true,
		 "stream_options": { "include_usage": true },
		 "messages": messages,
	});

	// Tools
	if !context.tools.is_empty() {
		let tools: Vec<Value> = context
			.tools
			.iter()
			.map(|t| {
				serde_json::json!({
					 "type": "function",
					 "function": {
						  "name": t.name,
						  "description": t.description,
						  "parameters": t.input_schema,
					 }
				})
			})
			.collect();
		body["tools"] = Value::Array(tools);
	}

	// Tool choice
	if let Some(tc) = crate::tool_choice::to_openai_completions(options.tool_choice.as_ref()) {
		body["tool_choice"] = tc;
	}

	// Temperature
	if let Some(temp) = options.temperature {
		body["temperature"] = serde_json::json!(temp);
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
	fn convert_user_text() {
		let msg =
			Message::User(UserMessage { content: vec![UserContent::Text { text: "hello".into() }] });
		let api = convert_message(&msg);
		assert_eq!(api["role"], "user");
		assert_eq!(api["content"], "hello");
	}

	#[test]
	fn convert_user_with_image() {
		let msg = Message::User(UserMessage {
			content: vec![UserContent::Text { text: "describe this".into() }, UserContent::Image {
				data:      "base64data".into(),
				mime_type: "image/png".into(),
			}],
		});
		let api = convert_message(&msg);
		let content = api["content"].as_array().unwrap();
		assert_eq!(content.len(), 2);
		assert_eq!(content[1]["type"], "image_url");
	}

	#[test]
	fn convert_assistant_with_tool_calls() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "I'll help.".into() },
				ContentBlock::ToolUse {
					id:    "call_1".into(),
					name:  "bash".into(),
					input: serde_json::json!({"command": "ls"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "assistant");
		assert!(api["tool_calls"].is_array());
	}

	#[test]
	fn convert_tool_result_to_tool_role() {
		let msg = Message::ToolResult(ToolResultMessage {
			tool_use_id: "call_1".into(),
			content:     vec![ToolResultContent::Text { text: "output".into() }],
			is_error:    false,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "tool");
		assert_eq!(api["tool_call_id"], "call_1");
	}

	#[test]
	fn build_request_includes_stream_options() {
		let model = test_model();
		let ctx = Context {
			system_prompt: Some(Arc::new("system".into())),
			messages:      vec![],
			tools:         vec![],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["stream"], true);
		assert_eq!(body["model"], "gpt-4o");
	}

	#[test]
	fn stop_reason_mapping() {
		assert_eq!(map_finish_reason(Some("stop")), StopReason::Stop);
		assert_eq!(map_finish_reason(Some("length")), StopReason::Length);
		assert_eq!(map_finish_reason(Some("tool_calls")), StopReason::ToolUse);
		assert_eq!(map_finish_reason(None), StopReason::Stop);
	}

	#[test]
	fn build_request_with_tools() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![],
			tools:         vec![crate::types::ToolDefinition {
				name:         "bash".into(),
				description:  "Run command".into(),
				input_schema: serde_json::json!({"type": "object"}),
			}],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		let tools = body["tools"].as_array().unwrap();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0]["type"], "function");
		assert_eq!(tools[0]["function"]["name"], "bash");
	}

	#[test]
	fn build_request_with_temperature() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions { temperature: Some(0.5), ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["temperature"], 0.5);
	}

	#[test]
	fn convert_assistant_thinking_as_text() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Thinking { thinking: "Let me think...".into() },
				ContentBlock::Text { text: "answer".into() },
			],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		});
		let api = convert_message(&msg);
		// Thinking blocks should be included in content
		let content = api["content"].as_str();
		// Should have "answer" at minimum - thinking handling may vary
		assert!(api["content"].is_string() || api["content"].is_array());
		// Verify thinking text is included (prepended as text)
		if let Some(text) = content {
			assert!(text.contains("answer"));
			assert!(text.contains("Let me think..."));
		}
	}

	#[test]
	fn provider_name() {
		let p = OpenAICompletionsProvider::new();
		assert_eq!(p.name(), "openai");
	}

	#[test]
	fn build_request_system_prompt_as_message() {
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
		let messages = body["messages"].as_array().unwrap();
		// First message should be system
		assert_eq!(messages[0]["role"], "system");
		assert_eq!(messages[0]["content"], "You are helpful.");
		// Second message should be user
		assert_eq!(messages[1]["role"], "user");
	}

	#[test]
	fn build_request_no_system_prompt() {
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
		let messages = body["messages"].as_array().unwrap();
		// No system message, first should be user
		assert_eq!(messages[0]["role"], "user");
	}

	#[test]
	fn build_request_max_tokens_override() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions { max_tokens: Some(4096), ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["max_tokens"], 4096);
	}

	#[test]
	fn build_request_uses_model_max_tokens_by_default() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["max_tokens"], 16384);
	}

	#[test]
	fn build_request_stream_options_include_usage() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["stream_options"]["include_usage"], true);
	}

	#[test]
	fn convert_assistant_text_only() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::Text { text: "Hello!".into() }],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "assistant");
		assert_eq!(api["content"], "Hello!");
		assert!(api.get("tool_calls").is_none());
	}

	#[test]
	fn convert_assistant_tool_calls_with_content() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me check.".into() },
				ContentBlock::ToolUse {
					id:    "call_abc".into(),
					name:  "bash".into(),
					input: serde_json::json!({"command": "ls"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "assistant");
		assert_eq!(api["content"], "Let me check.");
		let tool_calls = api["tool_calls"].as_array().unwrap();
		assert_eq!(tool_calls.len(), 1);
		assert_eq!(tool_calls[0]["id"], "call_abc");
		assert_eq!(tool_calls[0]["type"], "function");
		assert_eq!(tool_calls[0]["function"]["name"], "bash");
	}

	#[test]
	fn convert_assistant_tool_calls_without_text() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "call_abc".into(),
				name:  "bash".into(),
				input: serde_json::json!({"command": "ls"}),
			}],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "assistant");
		assert!(api["content"].is_null());
		assert!(api["tool_calls"].is_array());
	}

	#[test]
	fn convert_user_image_data_url_format() {
		let msg = Message::User(UserMessage {
			content: vec![UserContent::Text { text: "what is this?".into() }, UserContent::Image {
				data:      "abc123".into(),
				mime_type: "image/jpeg".into(),
			}],
		});
		let api = convert_message(&msg);
		let content = api["content"].as_array().unwrap();
		let image_url = content[1]["image_url"]["url"].as_str().unwrap();
		assert_eq!(image_url, "data:image/jpeg;base64,abc123");
	}

	#[test]
	fn convert_tool_result_content() {
		let msg = Message::ToolResult(ToolResultMessage {
			tool_use_id: "call_xyz".into(),
			content:     vec![ToolResultContent::Text { text: "file contents here".into() }],
			is_error:    false,
		});
		let api = convert_message(&msg);
		assert_eq!(api["role"], "tool");
		assert_eq!(api["tool_call_id"], "call_xyz");
		assert_eq!(api["content"], "file contents here");
	}

	#[test]
	fn stop_reason_content_filter() {
		assert_eq!(map_finish_reason(Some("content_filter")), StopReason::Error);
	}

	#[test]
	fn stop_reason_unknown_defaults_to_stop() {
		assert_eq!(map_finish_reason(Some("unknown_thing")), StopReason::Stop);
	}

	fn test_model() -> crate::models::Model {
		crate::models::Model {
			id:              "gpt-4o".into(),
			name:            "GPT-4o".into(),
			provider:        "openai".into(),
			api:             crate::models::Api::OpenAICompletions,
			base_url:        "https://api.openai.com".into(),
			reasoning:       false,
			supports_images: true,
			context_window:  128000,
			max_tokens:      16384,
			cost:            Default::default(),
		}
	}
}
