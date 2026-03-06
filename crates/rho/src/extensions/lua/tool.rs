use std::{path::Path, sync::Arc};

use mlua::{Function, LuaSerdeExt, RegistryKey, Table};
use rho_agent::tools::{OnToolUpdate, ToolOutput};
use tokio_util::sync::CancellationToken;

/// A tool backed by a Lua handler function.
pub struct LuaTool {
	pub name:         String,
	pub description:  String,
	pub input_schema: serde_json::Value,
	pub lua:          Arc<std::sync::Mutex<mlua::Lua>>,
	pub handler_key:  RegistryKey,
	pub ext_id:       String,
}

#[async_trait::async_trait]
impl rho_agent::tools::Tool for LuaTool {
	fn name(&self) -> &str {
		&self.name
	}

	fn description(&self) -> &str {
		&self.description
	}

	fn input_schema(&self) -> serde_json::Value {
		self.input_schema.clone()
	}

	async fn execute(
		&self,
		input: &serde_json::Value,
		cwd: &Path,
		_cancel: &CancellationToken,
		_on_update: Option<&OnToolUpdate>,
	) -> anyhow::Result<ToolOutput> {
		let lua = self
			.lua
			.lock()
			.map_err(|e| anyhow::anyhow!("[ext:{}] Lua mutex poisoned: {e}", self.ext_id))?;
		let func: Function = lua
			.registry_value(&self.handler_key)
			.map_err(|e| anyhow::anyhow!("[ext:{}] failed to get handler: {e}", self.ext_id))?;

		let input_val = lua.to_value(input).map_err(|e| {
			anyhow::anyhow!("[ext:{}] failed to convert input to Lua: {e}", self.ext_id)
		})?;

		let ctx_table: Table = lua
			.create_table()
			.map_err(|e| anyhow::anyhow!("[ext:{}] failed to create ctx table: {e}", self.ext_id))?;
		ctx_table
			.set("cwd", cwd.to_string_lossy().as_ref())
			.map_err(|e| anyhow::anyhow!("[ext:{}] failed to set cwd: {e}", self.ext_id))?;

		match func.call::<Table>((input_val, ctx_table)) {
			Ok(result) => {
				let content: String = result.get("content").unwrap_or_default();
				let is_error: bool = result.get("is_error").unwrap_or(false);
				Ok(ToolOutput { content, is_error })
			},
			Err(e) => Ok(ToolOutput {
				content:  format!("Extension error [{}]: {e}", self.ext_id),
				is_error: true,
			}),
		}
	}
}
