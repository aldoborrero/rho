use std::sync::{Arc, Mutex};

use mlua::{Function, Lua, LuaSerdeExt, RegistryKey, Result, Table, Value};

/// Collected registrations from a Lua `init(api)` call.
#[derive(Default)]
pub struct Registrations {
	pub tools:    Vec<ToolRegistration>,
	pub commands: Vec<CommandRegistration>,
	pub hooks:    HookRegistrations,
	pub context:  Option<String>,
}

pub struct ToolRegistration {
	pub name:         String,
	pub description:  String,
	pub input_schema: serde_json::Value,
	pub handler_key:  RegistryKey,
	pub concurrency:  rho_agent::tools::Concurrency,
}

pub struct CommandRegistration {
	pub name:        String,
	pub aliases:     Vec<String>,
	pub description: String,
	pub handler_key: RegistryKey,
}

#[derive(Default)]
pub struct HookRegistrations {
	pub before_tool_call: Option<RegistryKey>,
	pub after_tool_call:  Option<RegistryKey>,
	pub before_context:   Option<RegistryKey>,
	pub on_agent_event:   Option<RegistryKey>,
}

/// Build the `api` table passed to the Lua extension's init function.
///
/// Methods on the table collect registrations into the shared `Registrations`
/// struct, which is drained after the init call completes.
pub fn create_api_table(lua: &Lua, regs: Arc<Mutex<Registrations>>) -> Result<Table> {
	let api = lua.create_table()?;

	// api:register_tool({ name, description, input_schema, execute, concurrency? })
	{
		let regs = regs.clone();
		let register_tool = lua.create_function(move |lua, def: Table| {
			let name: String = def.get("name")?;
			let description: String = def.get("description")?;
			let schema_val: Value = def.get("input_schema")?;
			let input_schema: serde_json::Value = lua.from_value(schema_val)?;
			let execute: Function = def.get("execute")?;
			let handler_key = lua.create_registry_value(execute)?;

			let concurrency_str: Option<String> = def.get::<Option<String>>("concurrency")?.map(|s| s.to_lowercase());
			let concurrency = match concurrency_str.as_deref() {
				Some("exclusive") => rho_agent::tools::Concurrency::Exclusive,
				Some("shared") | None => rho_agent::tools::Concurrency::Shared,
				Some(other) => {
					eprintln!("[ext] unknown concurrency mode '{other}', defaulting to shared");
					rho_agent::tools::Concurrency::Shared
				},
			};

			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.tools
				.push(ToolRegistration { name, description, input_schema, handler_key, concurrency });
			Ok(())
		})?;
		api.set("register_tool", register_tool)?;
	}

	// api:on_before_tool_call(fn(name, id, input) -> { action, ... })
	{
		let regs = regs.clone();
		let on_before = lua.create_function(move |lua, func: Function| {
			let key = lua.create_registry_value(func)?;
			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.hooks
				.before_tool_call = Some(key);
			Ok(())
		})?;
		api.set("on_before_tool_call", on_before)?;
	}

	// api:on_after_tool_call(fn(name, id, content, is_error) -> { content?,
	// is_error? })
	{
		let regs = regs.clone();
		let on_after = lua.create_function(move |lua, func: Function| {
			let key = lua.create_registry_value(func)?;
			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.hooks
				.after_tool_call = Some(key);
			Ok(())
		})?;
		api.set("on_after_tool_call", on_after)?;
	}

	// api:on_before_context(fn(messages) -> { append_system_prompt?, inject_messages? })
	{
		let regs = regs.clone();
		let on_before_ctx = lua.create_function(move |lua, func: Function| {
			let key = lua.create_registry_value(func)?;
			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.hooks
				.before_context = Some(key);
			Ok(())
		})?;
		api.set("on_before_context", on_before_ctx)?;
	}

	// api:on_agent_event(fn(event) -> nil)
	{
		let regs = regs.clone();
		let on_event = lua.create_function(move |lua, func: Function| {
			let key = lua.create_registry_value(func)?;
			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.hooks
				.on_agent_event = Some(key);
			Ok(())
		})?;
		api.set("on_agent_event", on_event)?;
	}

	// api:register_command({ name, aliases?, description, execute })
	{
		let regs = regs.clone();
		let register_cmd = lua.create_function(move |lua, def: Table| {
			let name: String = def.get("name")?;
			let aliases: Vec<String> = def
				.get::<Option<Vec<String>>>("aliases")?
				.unwrap_or_default();
			let description: String = def.get("description")?;
			let execute: Function = def.get("execute")?;
			let handler_key = lua.create_registry_value(execute)?;

			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.commands
				.push(CommandRegistration { name, aliases, description, handler_key });
			Ok(())
		})?;
		api.set("register_command", register_cmd)?;
	}

	// api:set_context_provider(text)
	{
		let set_ctx = lua.create_function(move |_, text: String| {
			regs
				.lock()
				.map_err(|e| mlua::Error::external(format!("lock poisoned: {e}")))?
				.context = Some(text);
			Ok(())
		})?;
		api.set("set_context_provider", set_ctx)?;
	}

	Ok(api)
}
