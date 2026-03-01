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
// OpenAIResponsesProvider
// ---------------------------------------------------------------------------

/// Provider implementation for the OpenAI Responses API.
pub struct OpenAIResponsesProvider {
	client: Client,
}

impl OpenAIResponsesProvider {
	pub fn new() -> Self {
		Self { client: Client::new() }
	}
}

impl Default for OpenAIResponsesProvider {
	fn default() -> Self {
		Self::new()
	}
}

#[async_trait]
impl Provider for OpenAIResponsesProvider {
	fn name(&self) -> &str {
		"openai-responses"
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
	let url = format!("{}/v1/responses", model.base_url);
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
				message: format!("OpenAI Responses API returned {}: {}", status_code, body_text),
			},
			retryable,
			retry_after,
		);
		return;
	}

	// Process SSE stream using the shared parser (Responses API uses event: lines)
	let byte_stream = response.bytes_stream();
	let mut sse_stream = std::pin::pin!(parse_sse_stream(byte_stream));

	// State for accumulating the response
	let mut content_blocks: Vec<ContentBlock> = Vec::new();
	let mut stop_reason: Option<StopReason> = None;
	let mut usage = Usage::default();

	// Track content indices for stream events
	let mut next_content_index: usize = 0;

	// Track whether we have an open text block
	let mut text_started = false;
	let mut accumulated_text = String::new();

	// Track tool call state: maps item_id -> (call_id, name, accumulated_args,
	// content_index)
	let mut tool_calls: Vec<ToolCallState> = Vec::new();

	// Track whether we have any tool calls (for stop reason override)
	let mut has_tool_calls = false;

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
			Err(_) => continue,
		};

		match event_type.as_str() {
			"response.output_item.added" => {
				let item = &data["item"];
				let item_type = item["type"].as_str().unwrap_or("");

				match item_type {
					"message" => {
						// A message output item is being started, text will follow
						// via content_part.added and output_text.delta
					},
					"function_call" => {
						// Close any open text block first
						if text_started {
							content_blocks
								.push(ContentBlock::Text { text: std::mem::take(&mut accumulated_text) });
							writer.push(StreamEvent::TextEnd { content_index: next_content_index });
							next_content_index += 1;
							text_started = false;
						}

						let call_id = item["call_id"].as_str().unwrap_or("").to_string();
						let name = item["name"].as_str().unwrap_or("").to_string();
						let item_id = item["id"].as_str().unwrap_or("").to_string();

						let content_index = next_content_index;
						next_content_index += 1;

						tool_calls.push(ToolCallState {
							item_id,
							call_id: call_id.clone(),
							name: name.clone(),
							arguments: String::new(),
							content_index,
						});
						has_tool_calls = true;

						writer.push(StreamEvent::ToolCallStart { content_index, id: call_id, name });
					},
					"reasoning" => {
						// Reasoning output item
					},
					_ => {},
				}
			},

			"response.content_part.added" => {
				// A content part (e.g. output_text) is being added to a message item
				let part = &data["part"];
				let part_type = part["type"].as_str().unwrap_or("");

				if part_type == "output_text" && !text_started {
					text_started = true;
					writer.push(StreamEvent::TextStart { content_index: next_content_index });
				}
			},

			"response.output_text.delta" => {
				let delta = data["delta"].as_str().unwrap_or("");
				if !delta.is_empty() {
					if !text_started {
						text_started = true;
						writer.push(StreamEvent::TextStart { content_index: next_content_index });
					}
					accumulated_text.push_str(delta);
					writer.push(StreamEvent::TextDelta {
						content_index: next_content_index,
						text:          delta.to_string(),
					});
				}
			},

			"response.function_call_arguments.delta" => {
				let delta = data["delta"].as_str().unwrap_or("");
				if !delta.is_empty() {
					// Find the matching tool call by item_id
					let item_id = data["item_id"].as_str().unwrap_or("");
					if let Some(tc) = tool_calls.iter_mut().find(|tc| tc.item_id == item_id) {
						tc.arguments.push_str(delta);
						writer.push(StreamEvent::ToolCallDelta {
							content_index: tc.content_index,
							partial_json:  delta.to_string(),
						});
					}
				}
			},

			"response.output_item.done" => {
				let item = &data["item"];
				let item_type = item["type"].as_str().unwrap_or("");

				match item_type {
					"message" => {
						// Finalize any open text block
						if text_started {
							content_blocks
								.push(ContentBlock::Text { text: std::mem::take(&mut accumulated_text) });
							writer.push(StreamEvent::TextEnd { content_index: next_content_index });
							next_content_index += 1;
							text_started = false;
						}
					},
					"function_call" => {
						let item_id = item["id"].as_str().unwrap_or("");
						if let Some(tc) = tool_calls.iter().find(|tc| tc.item_id == item_id) {
							let input = if tc.arguments.is_empty() {
								Value::Null
							} else {
								serde_json::from_str(&tc.arguments).unwrap_or(Value::Null)
							};
							// Store composite ID: call_id|item_id
							let composite_id = format!("{}|{}", tc.call_id, tc.item_id);
							content_blocks.push(ContentBlock::ToolUse {
								id: composite_id,
								name: tc.name.clone(),
								input,
							});
							writer.push(StreamEvent::ToolCallEnd { content_index: tc.content_index });
						}
					},
					_ => {},
				}
			},

			"response.reasoning_summary_text.delta" => {
				let delta = data["delta"].as_str().unwrap_or("");
				if !delta.is_empty() {
					// Emit thinking events
					writer.push(StreamEvent::ThinkingDelta {
						content_index: next_content_index,
						thinking:      delta.to_string(),
					});
				}
			},

			"response.completed" => {
				let resp = &data["response"];

				// Extract usage
				if let Some(usage_obj) = resp.get("usage") {
					if let Some(input) = usage_obj["input_tokens"].as_u64() {
						usage.input_tokens = input as u32;
					}
					if let Some(output) = usage_obj["output_tokens"].as_u64() {
						usage.output_tokens = output as u32;
					}
				}

				// Extract status and map to stop reason
				let status_str = resp["status"].as_str().unwrap_or("completed");
				stop_reason = Some(if has_tool_calls && status_str == "completed" {
					StopReason::ToolUse
				} else {
					map_response_status(status_str)
				});

				// Finalize any open text block
				if text_started {
					content_blocks
						.push(ContentBlock::Text { text: std::mem::take(&mut accumulated_text) });
					writer.push(StreamEvent::TextEnd { content_index: next_content_index });
				}

				let message = AssistantMessage {
					content: content_blocks,
					stop_reason,
					usage: Some(usage.clone()),
				};
				writer.done(message, Some(usage));
				return;
			},

			"response.failed" => {
				let error_msg = data["response"]["status_details"]["error"]["message"]
					.as_str()
					.or_else(|| data["message"].as_str())
					.unwrap_or("Response failed")
					.to_string();
				let retryable = is_retryable(None, &error_msg);
				writer.error(StreamError { status: None, message: error_msg }, retryable, None);
				return;
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

	// Stream ended without response.completed
	// Assemble what we have
	if text_started {
		content_blocks.push(ContentBlock::Text { text: accumulated_text });
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
	item_id:       String,
	call_id:       String,
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

/// Convert a single `Message` to the OpenAI Responses API input format.
///
/// User messages use `{ "role": "user", "content": [{ "type": "input_text", ...
/// }] }`. Assistant messages produce `{ "role": "assistant", ... }` in the
/// input array. Tool results produce `{ "type": "function_call_output", ... }`
/// items.
///
/// For assistant messages and tool results that produce multiple items,
/// use `convert_assistant_to_items` and `convert_tool_result_to_items`
/// respectively.
pub(crate) fn convert_message(msg: &Message) -> Value {
	match msg {
		Message::User(user) => {
			let content: Vec<Value> = user
				.content
				.iter()
				.map(|c| match c {
					UserContent::Text { text } => {
						serde_json::json!({"type": "input_text", "text": text})
					},
					UserContent::Image { data, mime_type } => {
						serde_json::json!({
							 "type": "input_image",
							 "image_url": format!("data:{};base64,{}", mime_type, data),
						})
					},
				})
				.collect();
			serde_json::json!({"role": "user", "content": content})
		},
		Message::Assistant(_) => {
			// For the input array, we represent assistant turns as items.
			// Build a simplified representation.
			let mut items = convert_assistant_to_items(msg);
			if items.len() == 1 {
				items.remove(0)
			} else {
				// Return a wrapper; the caller should use convert_to_input_items instead
				serde_json::json!({"_items": items})
			}
		},
		Message::ToolResult(_) => {
			let items = convert_tool_result_to_items(msg);
			if items.len() == 1 {
				items[0].clone()
			} else {
				serde_json::json!({"_items": items})
			}
		},
	}
}

/// Convert an assistant message to OpenAI Responses API output items.
///
/// Text content produces a `message` item. ToolUse produces `function_call`
/// items. Composite tool call IDs (`call_id|item_id`) are split - `call_id` is
/// used for the `call_id` field.
pub(crate) fn convert_assistant_to_items(msg: &Message) -> Vec<Value> {
	let assistant = match msg {
		Message::Assistant(a) => a,
		_ => return vec![],
	};

	let mut items: Vec<Value> = Vec::new();
	let mut text_parts: Vec<String> = Vec::new();

	for block in &assistant.content {
		match block {
			ContentBlock::Text { text } => {
				text_parts.push(text.clone());
			},
			ContentBlock::Thinking { thinking } => {
				// Include thinking as text for context
				text_parts.push(thinking.clone());
			},
			ContentBlock::ToolUse { id, name, input } => {
				// Flush any accumulated text as a message item first
				if !text_parts.is_empty() {
					let combined_text = text_parts.join("\n");
					items.push(serde_json::json!({
						 "type": "message",
						 "role": "assistant",
						 "content": [{
							  "type": "output_text",
							  "text": combined_text,
						 }],
						 "status": "completed",
					}));
					text_parts.clear();
				}

				// Split composite ID: "call_id|item_id" -> use "call_id"
				let call_id = id.split('|').next().unwrap_or(id);
				items.push(serde_json::json!({
					 "type": "function_call",
					 "call_id": call_id,
					 "name": name,
					 "arguments": input.to_string(),
				}));
			},
		}
	}

	// Flush remaining text
	if !text_parts.is_empty() {
		let combined_text = text_parts.join("\n");
		items.push(serde_json::json!({
			 "type": "message",
			 "role": "assistant",
			 "content": [{
				  "type": "output_text",
				  "text": combined_text,
			 }],
			 "status": "completed",
		}));
	}

	items
}

/// Convert a tool result message to OpenAI Responses API function_call_output
/// items.
pub(crate) fn convert_tool_result_to_items(msg: &Message) -> Vec<Value> {
	let tool_result = match msg {
		Message::ToolResult(tr) => tr,
		_ => return vec![],
	};

	// Concatenate all text content
	let output: String = tool_result
		.content
		.iter()
		.filter_map(|c| match c {
			ToolResultContent::Text { text } => Some(text.as_str()),
			ToolResultContent::Image { .. } => None,
		})
		.collect::<Vec<&str>>()
		.join("\n");

	vec![serde_json::json!({
		 "type": "function_call_output",
		 "call_id": tool_result.tool_use_id,
		 "output": output,
	})]
}

/// Convert all messages to the OpenAI Responses API input array.
///
/// This flattens assistant messages and tool results that produce multiple
/// items into a single input array.
fn convert_to_input_items(messages: &[Message]) -> Vec<Value> {
	let mut input = Vec::new();

	for msg in messages {
		match msg {
			Message::User(_) => {
				input.push(convert_message(msg));
			},
			Message::Assistant(_) => {
				input.extend(convert_assistant_to_items(msg));
			},
			Message::ToolResult(_) => {
				input.extend(convert_tool_result_to_items(msg));
			},
		}
	}

	input
}

/// Map an OpenAI Responses API status string to our unified `StopReason`.
pub(crate) fn map_response_status(status: &str) -> StopReason {
	match status {
		"completed" => StopReason::Stop,
		"incomplete" => StopReason::Length,
		"failed" => StopReason::Error,
		"cancelled" => StopReason::Error,
		_ => StopReason::Stop,
	}
}

/// Map a `ReasoningLevel` to the OpenAI Responses API effort string.
pub(crate) fn map_reasoning_effort(level: &ReasoningLevel) -> &'static str {
	match level {
		ReasoningLevel::Minimal | ReasoningLevel::Low => "low",
		ReasoningLevel::Medium => "medium",
		ReasoningLevel::High | ReasoningLevel::XHigh => "high",
	}
}

/// Build the full OpenAI Responses API request body.
pub(crate) fn build_request_body(
	model: &Model,
	context: &Context,
	options: &StreamOptions,
) -> Value {
	let max_tokens = options.max_tokens.unwrap_or(model.max_tokens);

	let mut input = Vec::new();

	// System prompt as developer message (prepended to input)
	if let Some(ref system_prompt) = context.system_prompt {
		input.push(serde_json::json!({
			 "role": "developer",
			 "content": system_prompt,
		}));
	}

	// Conversation messages
	input.extend(convert_to_input_items(&context.messages));

	let mut body = serde_json::json!({
		 "model": model.id,
		 "input": input,
		 "stream": true,
		 "store": false,
		 "max_output_tokens": max_tokens,
	});

	// Tools
	if !context.tools.is_empty() {
		let tools: Vec<Value> = context
			.tools
			.iter()
			.map(|t| {
				serde_json::json!({
					 "type": "function",
					 "name": t.name,
					 "description": t.description,
					 "parameters": t.input_schema,
				})
			})
			.collect();
		body["tools"] = Value::Array(tools);
	}

	// Tool choice
	if let Some(tc) = crate::tool_choice::to_openai_responses(options.tool_choice.as_ref()) {
		body["tool_choice"] = tc;
	}

	// Temperature
	if let Some(temp) = options.temperature {
		body["temperature"] = serde_json::json!(temp);
	}

	// Reasoning effort
	if let Some(ref level) = options.reasoning {
		let effort = map_reasoning_effort(level);
		body["reasoning"] = serde_json::json!({
			 "effort": effort,
		});
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
		let content = api["content"].as_array().unwrap();
		assert_eq!(content[0]["type"], "input_text");
	}

	#[test]
	fn convert_user_with_image() {
		let msg = Message::User(UserMessage {
			content: vec![UserContent::Text { text: "what is this".into() }, UserContent::Image {
				data:      "base64data".into(),
				mime_type: "image/png".into(),
			}],
		});
		let api = convert_message(&msg);
		let content = api["content"].as_array().unwrap();
		assert_eq!(content.len(), 2);
		assert_eq!(content[1]["type"], "input_image");
	}

	#[test]
	fn convert_assistant_text() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::Text { text: "hello".into() }],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		});
		let items = convert_assistant_to_items(&msg);
		assert!(!items.is_empty());
		// Should produce a message output item
		assert_eq!(items[0]["type"], "message");
		assert_eq!(items[0]["role"], "assistant");
	}

	#[test]
	fn convert_assistant_with_tool_call() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text { text: "Let me check.".into() },
				ContentBlock::ToolUse {
					id:    "call_abc|item_xyz".into(),
					name:  "bash".into(),
					input: serde_json::json!({"command": "ls"}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let items = convert_assistant_to_items(&msg);
		// Should produce message + function_call items
		assert!(items.len() >= 2);
		let fc = items.iter().find(|i| i["type"] == "function_call").unwrap();
		assert_eq!(fc["name"], "bash");
		assert_eq!(fc["call_id"], "call_abc");
	}

	#[test]
	fn convert_tool_result() {
		let msg = Message::ToolResult(ToolResultMessage {
			tool_use_id: "call_abc".into(),
			content:     vec![ToolResultContent::Text { text: Arc::new("output".into()) }],
			is_error:    false,
		});
		let items = convert_tool_result_to_items(&msg);
		assert_eq!(items.len(), 1);
		assert_eq!(items[0]["type"], "function_call_output");
		assert_eq!(items[0]["call_id"], "call_abc");
	}

	#[test]
	fn build_request_basic() {
		let model = test_model();
		let ctx = Context {
			system_prompt: Some(Arc::new("system".into())),
			messages:      vec![Message::User(UserMessage {
				content: vec![UserContent::Text { text: "hi".into() }],
			})],
			tools:         vec![],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["model"], "o3");
		assert_eq!(body["stream"], true);
	}

	#[test]
	fn build_request_with_tools() {
		let model = test_model();
		let ctx = Context {
			system_prompt: None,
			messages:      vec![],
			tools:         vec![ToolDefinition {
				name:         "bash".into(),
				description:  "Run command".into(),
				input_schema: serde_json::json!({"type": "object"}),
			}],
		};
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		let tools = body["tools"].as_array().unwrap();
		assert_eq!(tools[0]["type"], "function");
		assert_eq!(tools[0]["name"], "bash");
	}

	#[test]
	fn build_request_with_reasoning() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions { reasoning: Some(ReasoningLevel::High), ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		assert!(body["reasoning"].is_object());
		assert_eq!(body["reasoning"]["effort"], "high");
	}

	#[test]
	fn status_to_stop_reason() {
		assert_eq!(map_response_status("completed"), StopReason::Stop);
		assert_eq!(map_response_status("incomplete"), StopReason::Length);
		assert_eq!(map_response_status("failed"), StopReason::Error);
	}

	#[test]
	fn reasoning_effort_mapping() {
		assert_eq!(map_reasoning_effort(&ReasoningLevel::Minimal), "low");
		assert_eq!(map_reasoning_effort(&ReasoningLevel::Low), "low");
		assert_eq!(map_reasoning_effort(&ReasoningLevel::Medium), "medium");
		assert_eq!(map_reasoning_effort(&ReasoningLevel::High), "high");
		assert_eq!(map_reasoning_effort(&ReasoningLevel::XHigh), "high");
	}

	#[test]
	fn provider_name() {
		let p = OpenAIResponsesProvider::new();
		assert_eq!(p.name(), "openai-responses");
	}

	#[test]
	fn build_request_store_false() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["store"], false);
	}

	#[test]
	fn build_request_max_output_tokens() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions { max_tokens: Some(4096), ..Default::default() };
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["max_output_tokens"], 4096);
	}

	#[test]
	fn build_request_uses_model_max_tokens_by_default() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert_eq!(body["max_output_tokens"], 100000);
	}

	#[test]
	fn build_request_system_as_developer() {
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
		let input = body["input"].as_array().unwrap();
		// First input should be developer message
		assert_eq!(input[0]["role"], "developer");
		assert_eq!(input[0]["content"], "You are helpful.");
		// Second input should be user
		assert_eq!(input[1]["role"], "user");
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
		let input = body["input"].as_array().unwrap();
		// First input should be user directly (no developer message)
		assert_eq!(input[0]["role"], "user");
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
	fn convert_user_image_url_format() {
		let msg = Message::User(UserMessage {
			content: vec![UserContent::Text { text: "what is this?".into() }, UserContent::Image {
				data:      "abc123".into(),
				mime_type: "image/jpeg".into(),
			}],
		});
		let api = convert_message(&msg);
		let content = api["content"].as_array().unwrap();
		let image_url = content[1]["image_url"].as_str().unwrap();
		assert_eq!(image_url, "data:image/jpeg;base64,abc123");
	}

	#[test]
	fn convert_tool_result_output_content() {
		let msg = Message::ToolResult(ToolResultMessage {
			tool_use_id: "call_xyz".into(),
			content:     vec![ToolResultContent::Text { text: Arc::new("file contents here".into()) }],
			is_error:    false,
		});
		let items = convert_tool_result_to_items(&msg);
		assert_eq!(items[0]["output"], "file contents here");
	}

	#[test]
	fn status_cancelled_maps_to_error() {
		assert_eq!(map_response_status("cancelled"), StopReason::Error);
	}

	#[test]
	fn status_unknown_maps_to_stop() {
		assert_eq!(map_response_status("something_unknown"), StopReason::Stop);
	}

	#[test]
	fn convert_assistant_text_only_produces_message_item() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::Text { text: "Hello!".into() }],
			stop_reason: Some(StopReason::Stop),
			usage:       None,
		});
		let items = convert_assistant_to_items(&msg);
		assert_eq!(items.len(), 1);
		assert_eq!(items[0]["type"], "message");
		let content = items[0]["content"].as_array().unwrap();
		assert_eq!(content[0]["type"], "output_text");
		assert_eq!(content[0]["text"], "Hello!");
	}

	#[test]
	fn convert_assistant_tool_only() {
		let msg = Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "call_abc".into(),
				name:  "bash".into(),
				input: serde_json::json!({"command": "ls"}),
			}],
			stop_reason: Some(StopReason::ToolUse),
			usage:       None,
		});
		let items = convert_assistant_to_items(&msg);
		assert_eq!(items.len(), 1);
		assert_eq!(items[0]["type"], "function_call");
		assert_eq!(items[0]["call_id"], "call_abc");
	}

	#[test]
	fn convert_to_input_items_flattens() {
		let messages = vec![
			Message::User(UserMessage { content: vec![UserContent::Text { text: "hi".into() }] }),
			Message::Assistant(AssistantMessage {
				content:     vec![
					ContentBlock::Text { text: "Let me check.".into() },
					ContentBlock::ToolUse {
						id:    "call_1|item_1".into(),
						name:  "bash".into(),
						input: serde_json::json!({"command": "ls"}),
					},
				],
				stop_reason: Some(StopReason::ToolUse),
				usage:       None,
			}),
			Message::ToolResult(ToolResultMessage {
				tool_use_id: "call_1".into(),
				content:     vec![ToolResultContent::Text { text: Arc::new("output".into()) }],
				is_error:    false,
			}),
		];
		let items = convert_to_input_items(&messages);
		// user + message + function_call + function_call_output = 4
		assert_eq!(items.len(), 4);
		assert_eq!(items[0]["role"], "user");
		assert_eq!(items[1]["type"], "message");
		assert_eq!(items[2]["type"], "function_call");
		assert_eq!(items[3]["type"], "function_call_output");
	}

	#[test]
	fn build_request_all_reasoning_levels() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };

		for (level, expected) in [
			(ReasoningLevel::Minimal, "low"),
			(ReasoningLevel::Low, "low"),
			(ReasoningLevel::Medium, "medium"),
			(ReasoningLevel::High, "high"),
			(ReasoningLevel::XHigh, "high"),
		] {
			let options = StreamOptions { reasoning: Some(level), ..Default::default() };
			let body = build_request_body(&model, &ctx, &options);
			assert_eq!(body["reasoning"]["effort"], expected);
		}
	}

	#[test]
	fn build_request_no_reasoning_when_none() {
		let model = test_model();
		let ctx = Context { system_prompt: None, messages: vec![], tools: vec![] };
		let options = StreamOptions::default();
		let body = build_request_body(&model, &ctx, &options);
		assert!(body.get("reasoning").is_none());
	}

	fn test_model() -> crate::models::Model {
		crate::models::Model {
			id:              "o3".into(),
			name:            "O3".into(),
			provider:        "openai".into(),
			api:             crate::models::Api::OpenAIResponses,
			base_url:        "https://api.openai.com".into(),
			reasoning:       true,
			supports_images: true,
			context_window:  200000,
			max_tokens:      100000,
			cost:            Default::default(),
		}
	}
}
