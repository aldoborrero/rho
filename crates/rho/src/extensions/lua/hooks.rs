use std::sync::Arc;

use mlua::{Function, LuaSerdeExt, RegistryKey, Table};
use rho_agent::hooks::{ToolCallAction, ToolResultModification};

/// Hook dispatcher backed by Lua handler functions.
pub struct LuaHooks {
	pub lua:                  Arc<std::sync::Mutex<mlua::Lua>>,
	pub before_tool_call_key: Option<RegistryKey>,
	pub after_tool_call_key:  Option<RegistryKey>,
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
}
