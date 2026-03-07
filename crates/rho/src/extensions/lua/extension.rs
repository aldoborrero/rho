use std::sync::Arc;

use mlua::Function;
use rho_agent::{hooks::AgentHooks, tools::Tool};

use super::{hooks::LuaHooks, runtime::LuaRuntime, tool::LuaTool};
use crate::{
	commands::CommandResult,
	extensions::types::{DynamicCommand, Extension, ExtensionManifest},
};

/// An extension loaded from a Lua script.
pub struct LuaExtension {
	manifest: ExtensionManifest,
	runtime:  LuaRuntime,
}

impl LuaExtension {
	pub const fn new(manifest: ExtensionManifest, runtime: LuaRuntime) -> Self {
		Self { manifest, runtime }
	}

	/// Duplicate a `RegistryKey` by reading the value and re-registering it.
	/// This creates an independent key that can be dropped separately.
	fn dup_registry_key(
		lua: &mlua::Lua,
		key: &mlua::RegistryKey,
	) -> anyhow::Result<mlua::RegistryKey> {
		let func: Function = lua.registry_value(key)?;
		Ok(lua.create_registry_value(func)?)
	}
}

impl Extension for LuaExtension {
	fn id(&self) -> &str {
		&self.manifest.id
	}

	fn name(&self) -> &str {
		&self.manifest.name
	}

	fn tools(&self) -> Vec<Box<dyn Tool>> {
		let Ok(lua) = self.runtime.lua.lock() else {
			eprintln!("[ext:{}] Lua mutex poisoned in tools()", self.manifest.id);
			return Vec::new();
		};
		self
			.runtime
			.registrations
			.tools
			.iter()
			.filter_map(|reg| {
				let new_key = match Self::dup_registry_key(&lua, &reg.handler_key) {
					Ok(k) => k,
					Err(e) => {
						eprintln!(
							"[ext:{}] failed to dup tool key for {}: {e}",
							self.manifest.id, reg.name
						);
						return None;
					},
				};
				Some(Box::new(LuaTool {
					name:         reg.name.clone(),
					description:  reg.description.clone(),
					input_schema: reg.input_schema.clone(),
					lua:          self.runtime.lua.clone(),
					handler_key:  new_key,
					ext_id:       self.manifest.id.clone(),
					concurrency:  reg.concurrency,
				}) as Box<dyn Tool>)
			})
			.collect()
	}

	fn commands(&self) -> Vec<DynamicCommand> {
		let Ok(lua_guard) = self.runtime.lua.lock() else {
			eprintln!("[ext:{}] Lua mutex poisoned in commands()", self.manifest.id);
			return Vec::new();
		};
		self
			.runtime
			.registrations
			.commands
			.iter()
			.filter_map(|reg| {
				let new_key = match Self::dup_registry_key(&lua_guard, &reg.handler_key) {
					Ok(k) => k,
					Err(e) => {
						eprintln!(
							"[ext:{}] failed to dup command key for {}: {e}",
							self.manifest.id, reg.name
						);
						return None;
					},
				};
				// Wrap the key in an Arc so the closure can hold a shared reference.
				let key = Arc::new(new_key);
				let lua = self.runtime.lua.clone();
				let ext_id = self.manifest.id.clone();

				Some(DynamicCommand {
					name:        reg.name.clone(),
					aliases:     reg.aliases.clone(),
					description: reg.description.clone(),
					handler:     Box::new(move |args: &str| {
						let lua = lua
							.lock()
							.map_err(|e| anyhow::anyhow!("[ext:{ext_id}] Lua mutex poisoned: {e}"))?;
						let func: Function = lua
							.registry_value(&key)
							.map_err(|e| anyhow::anyhow!("[ext:{ext_id}] command handler error: {e}"))?;
						let result: String = func
							.call(args.to_owned())
							.map_err(|e| anyhow::anyhow!("[ext:{ext_id}] command error: {e}"))?;
						Ok(CommandResult::Message(result))
					}),
				})
			})
			.collect()
	}

	fn hooks(&self) -> Option<Arc<dyn AgentHooks>> {
		let has_before = self.runtime.registrations.hooks.before_tool_call.is_some();
		let has_after = self.runtime.registrations.hooks.after_tool_call.is_some();
		let has_before_ctx = self.runtime.registrations.hooks.before_context.is_some();
		let has_on_event = self.runtime.registrations.hooks.on_agent_event.is_some();

		if !has_before && !has_after && !has_before_ctx && !has_on_event {
			return None;
		}

		let Ok(lua_guard) = self.runtime.lua.lock() else {
			eprintln!("[ext:{}] Lua mutex poisoned in hooks()", self.manifest.id);
			return None;
		};

		let before_key = if let Some(ref key) = self.runtime.registrations.hooks.before_tool_call {
			match Self::dup_registry_key(&lua_guard, key) {
				Ok(k) => Some(k),
				Err(e) => {
					eprintln!("[ext:{}] failed to dup before_tool_call key: {e}", self.manifest.id);
					None
				},
			}
		} else {
			None
		};

		let after_key = if let Some(ref key) = self.runtime.registrations.hooks.after_tool_call {
			match Self::dup_registry_key(&lua_guard, key) {
				Ok(k) => Some(k),
				Err(e) => {
					eprintln!("[ext:{}] failed to dup after_tool_call key: {e}", self.manifest.id);
					None
				},
			}
		} else {
			None
		};

		let before_ctx_key = if let Some(ref key) = self.runtime.registrations.hooks.before_context {
			match Self::dup_registry_key(&lua_guard, key) {
				Ok(k) => Some(k),
				Err(e) => {
					eprintln!("[ext:{}] failed to dup before_context key: {e}", self.manifest.id);
					None
				},
			}
		} else {
			None
		};

		let on_event_key = if let Some(ref key) = self.runtime.registrations.hooks.on_agent_event {
			match Self::dup_registry_key(&lua_guard, key) {
				Ok(k) => Some(k),
				Err(e) => {
					eprintln!("[ext:{}] failed to dup on_agent_event key: {e}", self.manifest.id);
					None
				},
			}
		} else {
			None
		};

		drop(lua_guard);

		if before_key.is_none()
			&& after_key.is_none()
			&& before_ctx_key.is_none()
			&& on_event_key.is_none()
		{
			return None;
		}

		Some(Arc::new(LuaHooks {
			lua:                  self.runtime.lua.clone(),
			before_tool_call_key: before_key,
			after_tool_call_key:  after_key,
			before_context_key:   before_ctx_key,
			on_agent_event_key:   on_event_key,
			ext_id:               self.manifest.id.clone(),
		}))
	}

	fn context_provider(&self) -> Option<String> {
		self.runtime.registrations.context.clone()
	}
}
