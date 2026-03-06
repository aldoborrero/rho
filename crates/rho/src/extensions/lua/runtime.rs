use std::{
	path::Path,
	sync::{Arc, Mutex},
};

use mlua::{Function, Lua};

use super::bridge::{self, Registrations};

/// Manages a sandboxed Luau instance and the registrations collected during
/// init.
///
/// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because Lua calls are
/// sub-millisecond — no benefit from async locking, and it avoids
/// "cannot block inside async runtime" issues in sync trait methods.
pub struct LuaRuntime {
	pub lua:           Arc<Mutex<Lua>>,
	pub registrations: Registrations,
}

impl LuaRuntime {
	/// Create a new Lua runtime, execute the extension entry script, and
	/// collect all registrations (tools, hooks, commands, context).
	pub fn new(ext_id: &str, ext_dir: &Path, entry_file: &str) -> anyhow::Result<Self> {
		let lua = Lua::new();
		lua.sandbox(true)
			.map_err(|e| anyhow::anyhow!("failed to enable Luau sandbox: {e}"))?;

		// Override print() to route to stderr with extension prefix.
		let id_owned = ext_id.to_owned();
		let print_fn = lua
			.create_function(move |_, args: mlua::Variadic<String>| {
				let msg = args.into_iter().collect::<Vec<_>>().join("\t");
				eprintln!("[ext:{id_owned}] {msg}");
				Ok(())
			})
			.map_err(|e| anyhow::anyhow!("failed to create print override: {e}"))?;
		lua.globals()
			.set("print", print_fn)
			.map_err(|e| anyhow::anyhow!("failed to set print: {e}"))?;

		// Read and evaluate the entry script.
		let entry_path = ext_dir.join(entry_file);
		let source = std::fs::read_to_string(&entry_path)
			.map_err(|e| anyhow::anyhow!("failed to read {}: {e}", entry_path.display()))?;

		let init_fn: Function = lua
			.load(&source)
			.set_name(entry_file)
			.eval()
			.map_err(|e| anyhow::anyhow!("failed to evaluate {entry_file}: {e}"))?;

		// Build the api table and call init(api).
		let regs = Arc::new(Mutex::new(Registrations::default()));
		let api = bridge::create_api_table(&lua, regs.clone())
			.map_err(|e| anyhow::anyhow!("failed to create api table: {e}"))?;

		init_fn
			.call::<()>(api)
			.map_err(|e| anyhow::anyhow!("init() failed: {e}"))?;

		// Drain registrations. The Arc has extra references held by closures
		// stored in the Lua state, so we take() from the Mutex instead.
		let registrations = std::mem::take(
			&mut *regs
				.lock()
				.map_err(|e| anyhow::anyhow!("registrations mutex poisoned: {e}"))?,
		);

		Ok(Self { lua: Arc::new(Mutex::new(lua)), registrations })
	}
}
