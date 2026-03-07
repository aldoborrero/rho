use std::sync::Arc;

use mlua::{Function, LuaSerdeExt, RegistryKey, Table};
use rho_agent::{
	events::{AgentEvent, AgentOutcome},
	hooks::{ContextModification, ToolCallAction, ToolResultModification},
	types::Message,
};

/// Hook dispatcher backed by Lua handler functions.
pub struct LuaHooks {
	pub lua:                  Arc<std::sync::Mutex<mlua::Lua>>,
	pub before_tool_call_key: Option<RegistryKey>,
	pub after_tool_call_key:  Option<RegistryKey>,
	pub before_context_key:   Option<RegistryKey>,
	pub on_agent_event_key:   Option<RegistryKey>,
	pub ext_id:               String,
}

#[async_trait::async_trait]
impl rho_agent::hooks::AgentHooks for LuaHooks {
	async fn before_tool_call(
		&self,
		name: &str,
		id: &str,
		input: &serde_json::Value,
	) -> anyhow::Result<ToolCallAction> {
		let Some(key) = &self.before_tool_call_key else {
			return Ok(ToolCallAction::Continue);
		};

		let lua = self
			.lua
			.lock()
			.map_err(|e| anyhow::anyhow!("[ext:{}] Lua mutex poisoned: {e}", self.ext_id))?;
		let func: Function = match lua.registry_value(key) {
			Ok(f) => f,
			Err(e) => {
				eprintln!("[ext:{}] before_tool_call handler error: {e}", self.ext_id);
				return Ok(ToolCallAction::Continue);
			},
		};

		let input_val = lua.to_value(input)?;
		let result: Table = match func.call((name.to_owned(), id.to_owned(), input_val)) {
			Ok(t) => t,
			Err(e) => {
				eprintln!("[ext:{}] before_tool_call error: {e}", self.ext_id);
				return Ok(ToolCallAction::Continue);
			},
		};

		let action: String = result.get("action").unwrap_or_default();
		match action.as_str() {
			"block" => {
				let reason: String = result
					.get("reason")
					.unwrap_or_else(|_| "blocked by extension".to_owned());
				Ok(ToolCallAction::Block { reason })
			},
			"modify_input" => {
				let modified_val: mlua::Value = result.get("input")?;
				let modified: serde_json::Value = lua.from_value(modified_val)?;
				Ok(ToolCallAction::ModifyInput { input: modified })
			},
			_ => Ok(ToolCallAction::Continue),
		}
	}

	async fn after_tool_call(
		&self,
		name: &str,
		id: &str,
		content: &str,
		is_error: bool,
	) -> anyhow::Result<Option<ToolResultModification>> {
		let Some(key) = &self.after_tool_call_key else {
			return Ok(None);
		};

		let lua = self
			.lua
			.lock()
			.map_err(|e| anyhow::anyhow!("[ext:{}] Lua mutex poisoned: {e}", self.ext_id))?;
		let func: Function = match lua.registry_value(key) {
			Ok(f) => f,
			Err(e) => {
				eprintln!("[ext:{}] after_tool_call handler error: {e}", self.ext_id);
				return Ok(None);
			},
		};

		let result: Table =
			match func.call((name.to_owned(), id.to_owned(), content.to_owned(), is_error)) {
				Ok(t) => t,
				Err(e) => {
					eprintln!("[ext:{}] after_tool_call error: {e}", self.ext_id);
					return Ok(None);
				},
			};

		let mod_content: Option<String> = result.get("content").ok();
		let mod_is_error: Option<bool> = result.get("is_error").ok();

		if mod_content.is_some() || mod_is_error.is_some() {
			Ok(Some(ToolResultModification { content: mod_content, is_error: mod_is_error }))
		} else {
			Ok(None)
		}
	}

	async fn before_context(
		&self,
		messages: &[Message],
	) -> anyhow::Result<Option<ContextModification>> {
		let Some(key) = &self.before_context_key else {
			return Ok(None);
		};

		let lua = self
			.lua
			.lock()
			.map_err(|e| anyhow::anyhow!("[ext:{}] Lua mutex poisoned: {e}", self.ext_id))?;
		let func: Function = match lua.registry_value(key) {
			Ok(f) => f,
			Err(e) => {
				eprintln!("[ext:{}] before_context handler error: {e}", self.ext_id);
				return Ok(None);
			},
		};

		// Convert messages to a Lua-friendly summary (array of {role, content}).
		let msgs_table = lua.create_table()?;
		for (i, msg) in messages.iter().enumerate() {
			let entry = lua.create_table()?;
			match msg {
				Message::User(u) => {
					entry.set("role", "user")?;
					entry.set("content", u.content.as_str())?;
				},
				Message::Assistant(a) => {
					entry.set("role", "assistant")?;
					// Extract first text block as content summary.
					let text = a
						.content
						.iter()
						.find_map(|b| match b {
							rho_agent::types::ContentBlock::Text { text } => Some(text.as_str()),
							_ => None,
						})
						.unwrap_or("");
					entry.set("content", text)?;
				},
				Message::ToolResult(t) => {
					entry.set("role", "tool_result")?;
					entry.set("content", t.content.as_str())?;
				},
				Message::BashExecution(b) => {
					entry.set("role", "bash_execution")?;
					entry.set("content", b.command.as_str())?;
					entry.set("output", b.output.as_str())?;
				},
			}
			msgs_table.set(i + 1, entry)?;
		}

		let result: Table = match func.call(msgs_table) {
			Ok(t) => t,
			Err(e) => {
				eprintln!("[ext:{}] before_context error: {e}", self.ext_id);
				return Ok(None);
			},
		};

		let append: Option<String> = result.get("append_system_prompt").ok();

		// Parse inject_messages: array of {role = "user", content = "..."}.
		// Only user messages are supported from Lua.
		let mut inject: Vec<Message> = Vec::new();
		if let Ok(msgs) = result.get::<Table>("inject_messages") {
			for pair in msgs.pairs::<i64, Table>() {
				if let Ok((_, entry)) = pair {
					if let Ok(content) = entry.get::<String>("content") {
						inject.push(Message::User(rho_agent::types::UserMessage { content }));
					}
				}
			}
		}

		if append.is_some() || !inject.is_empty() {
			Ok(Some(ContextModification {
				append_system_prompt: append,
				inject_messages:      inject,
			}))
		} else {
			Ok(None)
		}
	}

	async fn on_agent_event(&self, event: &AgentEvent) {
		let Some(key) = &self.on_agent_event_key else {
			return;
		};
		let Ok(lua) = self.lua.lock() else {
			return;
		};
		let Ok(func) = lua.registry_value::<Function>(key) else {
			return;
		};
		let table = match event_to_lua_table(&lua, event) {
			Ok(t) => t,
			Err(e) => {
				eprintln!("[ext:{}] event_to_lua_table error: {e}", self.ext_id);
				return;
			},
		};
		if let Err(e) = func.call::<()>(table) {
			eprintln!("[ext:{}] on_agent_event error: {e}", self.ext_id);
		}
	}
}

/// Convert an `AgentEvent` to a flat Lua table for extension consumption.
fn event_to_lua_table(lua: &mlua::Lua, event: &AgentEvent) -> mlua::Result<Table> {
	let t = lua.create_table()?;
	match event {
		AgentEvent::AgentStart => {
			t.set("type", "agent_start")?;
		},
		AgentEvent::TurnStart { turn } => {
			t.set("type", "turn_start")?;
			t.set("turn", *turn)?;
		},
		AgentEvent::MessageStart => {
			t.set("type", "message_start")?;
		},
		AgentEvent::TextDelta(s) => {
			t.set("type", "text_delta")?;
			t.set("content", s.as_str())?;
		},
		AgentEvent::ThinkingDelta(s) => {
			t.set("type", "thinking_delta")?;
			t.set("content", s.as_str())?;
		},
		AgentEvent::ToolCallStart { id, name, input } => {
			t.set("type", "tool_call_start")?;
			t.set("id", id.as_str())?;
			t.set("name", name.as_str())?;
			let input_val = lua.to_value(input.as_ref())?;
			t.set("input", input_val)?;
		},
		AgentEvent::ToolExecutionUpdate { id, name, content } => {
			t.set("type", "tool_execution_update")?;
			t.set("id", id.as_str())?;
			t.set("name", name.as_str())?;
			t.set("content", content.as_str())?;
		},
		AgentEvent::ToolCallResult { id, name, content, is_error } => {
			t.set("type", "tool_call_result")?;
			t.set("id", id.as_str())?;
			t.set("name", name.as_str())?;
			t.set("content", content.as_str())?;
			t.set("is_error", *is_error)?;
		},
		AgentEvent::TurnEnd { .. } => {
			t.set("type", "turn_end")?;
		},
		AgentEvent::MessageComplete(_) => {
			t.set("type", "message_complete")?;
		},
		AgentEvent::SteeringProcessed { .. } => {
			t.set("type", "steering_processed")?;
		},
		AgentEvent::RetryScheduled { attempt, delay_ms, error } => {
			t.set("type", "retry_scheduled")?;
			t.set("attempt", *attempt)?;
			t.set("delay_ms", *delay_ms)?;
			t.set("error", error.as_str())?;
		},
		AgentEvent::Done { outcome, .. } => {
			t.set("type", "done")?;
			let outcome_str = match outcome {
				AgentOutcome::Stop { .. } => "stop",
				AgentOutcome::MaxTokens { .. } => "max_tokens",
				AgentOutcome::Failed { .. } => "failed",
				AgentOutcome::Cancelled => "cancelled",
			};
			t.set("outcome", outcome_str)?;
		},
	}
	Ok(t)
}
