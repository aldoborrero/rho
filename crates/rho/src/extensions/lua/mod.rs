mod bridge;
mod extension;
mod hooks;
mod runtime;
mod tool;

use std::path::PathBuf;

use super::types::{Extension, ExtensionManifest};

/// Load a Lua-based extension from its manifest and entry file.
///
/// Creates a sandboxed Luau runtime, executes the entry script's `init(api)`
/// function, and collects registered tools, hooks, commands, and context
/// providers.
pub fn load_lua_extension(
	manifest: ExtensionManifest,
	ext_dir: PathBuf,
	entry_file: String,
) -> anyhow::Result<Box<dyn Extension>> {
	let runtime = runtime::LuaRuntime::new(&manifest.id, &ext_dir, &entry_file)?;
	Ok(Box::new(extension::LuaExtension::new(manifest, runtime)))
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use rho_agent::hooks::ToolCallAction;

	use super::*;
	use crate::extensions::types::RuntimeConfig;

	/// Helper: create a manifest for testing.
	fn test_manifest(id: &str) -> ExtensionManifest {
		ExtensionManifest {
			id:          id.to_owned(),
			name:        format!("Test {id}"),
			version:     "0.1.0".to_owned(),
			description: String::new(),
			runtime:     Some(RuntimeConfig {
				engine: "lua".to_owned(),
				entry:  "main.lua".to_owned(),
			}),
		}
	}

	/// Helper: write a Lua file into a temp dir and load the extension.
	fn load_test_ext(lua_source: &str) -> anyhow::Result<Box<dyn Extension>> {
		let dir = tempfile::tempdir()?;
		std::fs::write(dir.path().join("main.lua"), lua_source)?;
		let manifest = test_manifest("test-ext");
		let ext = load_lua_extension(manifest, dir.path().to_owned(), "main.lua".to_owned())?;
		// Leak dir so it isn't cleaned up while extension holds references.
		std::mem::forget(dir);
		Ok(ext)
	}

	// --- Manifest backward compat ---

	#[test]
	fn manifest_without_runtime_parses_as_none() {
		let toml_str = r#"
id = "legacy"
name = "Legacy Extension"
version = "1.0.0"
"#;
		let manifest: ExtensionManifest = toml::from_str(toml_str).unwrap();
		assert!(manifest.runtime.is_none());
	}

	#[test]
	fn manifest_with_runtime_parses_correctly() {
		let toml_str = r#"
id = "lua-ext"
name = "Lua Extension"
version = "1.0.0"

[runtime]
engine = "lua"
"#;
		let manifest: ExtensionManifest = toml::from_str(toml_str).unwrap();
		let rt = manifest.runtime.unwrap();
		assert_eq!(rt.engine, "lua");
		assert_eq!(rt.entry, "main.lua"); // default
	}

	#[test]
	fn manifest_with_custom_entry() {
		let toml_str = r#"
id = "custom"
name = "Custom"
version = "1.0.0"

[runtime]
engine = "lua"
entry = "init.lua"
"#;
		let manifest: ExtensionManifest = toml::from_str(toml_str).unwrap();
		assert_eq!(manifest.runtime.unwrap().entry, "init.lua");
	}

	// --- Load + execute tool ---

	#[tokio::test]
	async fn load_and_execute_tool() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "reverse",
        description = "Reverses a string",
        input_schema = { type = "object", properties = { text = { type = "string" } }, required = { "text" } },
        execute = function(input, ctx)
            return { content = string.reverse(input.text), is_error = false }
        end
    })
end
"#,
		)
		.unwrap();

		let tools = ext.tools();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].name(), "reverse");
		assert_eq!(tools[0].description(), "Reverses a string");

		let cancel = tokio_util::sync::CancellationToken::new();
		let input = serde_json::json!({"text": "hello"});
		let result = tools[0]
			.execute(&input, Path::new("/tmp"), &cancel, None)
			.await
			.unwrap();
		assert_eq!(result.content, "olleh");
		assert!(!result.is_error);
	}

	// --- Hook dispatch: before_tool_call returning block ---

	#[tokio::test]
	async fn hook_before_tool_call_block() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.on_before_tool_call(function(name, id, input)
        if name == "bash" then
            return { action = "block", reason = "no shell allowed" }
        end
        return { action = "continue" }
    end)
end
"#,
		)
		.unwrap();

		let hooks = ext.hooks().expect("should have hooks");
		let action = hooks
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		match action {
			ToolCallAction::Block { reason } => assert_eq!(reason, "no shell allowed"),
			_ => panic!("expected Block"),
		}

		// Non-bash tool should continue.
		let action = hooks
			.before_tool_call("read", "id_2", &serde_json::json!({}))
			.await
			.unwrap();
		assert!(matches!(action, ToolCallAction::Continue));
	}

	// --- Hook dispatch: before_tool_call returning modify_input ---

	#[tokio::test]
	async fn hook_before_tool_call_modify_input() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.on_before_tool_call(function(name, id, input)
        input.injected = "yes"
        return { action = "modify_input", input = input }
    end)
end
"#,
		)
		.unwrap();

		let hooks = ext.hooks().expect("should have hooks");
		let action = hooks
			.before_tool_call("bash", "id_1", &serde_json::json!({"cmd": "ls"}))
			.await
			.unwrap();
		match action {
			ToolCallAction::ModifyInput { input } => {
				assert_eq!(input["injected"], "yes");
				assert_eq!(input["cmd"], "ls");
			},
			_ => panic!("expected ModifyInput"),
		}
	}

	// --- Hook dispatch: after_tool_call ---

	#[tokio::test]
	async fn hook_after_tool_call_modifies_result() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.on_after_tool_call(function(name, id, content, is_error)
        return { content = "modified: " .. content }
    end)
end
"#,
		)
		.unwrap();

		let hooks = ext.hooks().expect("should have hooks");
		let modification = hooks
			.after_tool_call("bash", "id_1", "original output", false)
			.await
			.unwrap();
		let m = modification.unwrap();
		assert_eq!(m.content.unwrap(), "modified: original output");
	}

	// --- Error isolation: syntax error in Lua ---

	#[test]
	fn lua_syntax_error_returns_err() {
		let result = load_test_ext("this is not valid lua !!!@@@");
		let err = match result {
			Err(e) => e.to_string(),
			Ok(_) => panic!("expected error for invalid Lua syntax"),
		};
		assert!(err.contains("failed to evaluate"), "error should mention evaluation failure: {err}");
	}

	// --- Error isolation: init function runtime error ---

	#[test]
	fn lua_runtime_error_returns_err() {
		let result = load_test_ext(
			r#"
return function(api)
    error("intentional crash")
end
"#,
		);
		let err = match result {
			Err(e) => e.to_string(),
			Ok(_) => panic!("expected error for runtime crash"),
		};
		assert!(err.contains("init() failed"), "error should mention init failure: {err}");
	}

	// --- Sandbox: io/os access blocked ---

	#[test]
	fn sandbox_blocks_io_access() {
		let result = load_test_ext(
			r#"
return function(api)
    io.open("/etc/passwd", "r")
end
"#,
		);
		assert!(result.is_err(), "io.open should be blocked by Luau sandbox");
	}

	#[test]
	fn sandbox_blocks_os_execute() {
		let result = load_test_ext(
			r#"
return function(api)
    os.execute("echo pwned")
end
"#,
		);
		assert!(result.is_err(), "os.execute should be blocked by Luau sandbox");
	}

	// --- Context provider ---

	#[test]
	fn context_provider_returns_set_text() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.set_context_provider("Extra context from test extension")
end
"#,
		)
		.unwrap();
		assert_eq!(ext.context_provider().unwrap(), "Extra context from test extension");
	}

	#[test]
	fn no_context_provider_returns_none() {
		let ext = load_test_ext(
			r#"
return function(api)
    -- register nothing
end
"#,
		)
		.unwrap();
		assert!(ext.context_provider().is_none());
	}

	// --- No hooks returns None ---

	#[test]
	fn no_hooks_returns_none() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.set_context_provider("just context")
end
"#,
		)
		.unwrap();
		assert!(ext.hooks().is_none());
	}

	// --- Command registration ---

	#[test]
	fn command_registration_and_dispatch() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_command({
        name = "greet",
        description = "Say hello",
        execute = function(args)
            return "Hello, " .. args .. "!"
        end
    })
end
"#,
		)
		.unwrap();

		let commands = ext.commands();
		assert_eq!(commands.len(), 1);
		assert_eq!(commands[0].name, "greet");
		assert_eq!(commands[0].description, "Say hello");

		let result = (commands[0].handler)("world").unwrap();
		match result {
			crate::commands::CommandResult::Message(msg) => assert_eq!(msg, "Hello, world!"),
			_ => panic!("expected Message"),
		}
	}

	// --- Hook dispatch: before_context ---

	#[tokio::test]
	async fn hook_before_context_appends_prompt() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.on_before_context(function(messages)
        return { append_system_prompt = "extra context from lua" }
    end)
end
"#,
		)
		.unwrap();

		let hooks = ext.hooks().expect("should have hooks");
		let modification = hooks.before_context(&[]).await.unwrap();
		let m = modification.unwrap();
		assert_eq!(m.append_system_prompt.unwrap(), "extra context from lua");
		assert!(m.inject_messages.is_empty());
	}

	#[tokio::test]
	async fn hook_before_context_injects_messages() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.on_before_context(function(messages)
        return {
            append_system_prompt = "added",
            inject_messages = {
                { role = "user", content = "injected msg" }
            }
        }
    end)
end
"#,
		)
		.unwrap();

		let hooks = ext.hooks().expect("should have hooks");
		let modification = hooks.before_context(&[]).await.unwrap();
		let m = modification.unwrap();
		assert_eq!(m.append_system_prompt.unwrap(), "added");
		assert_eq!(m.inject_messages.len(), 1);
		match &m.inject_messages[0] {
			rho_agent::types::Message::User(u) => assert_eq!(u.content, "injected msg"),
			_ => panic!("expected User message"),
		}
	}

	// --- Hook dispatch: on_agent_event ---

	#[tokio::test]
	async fn hook_on_agent_event_receives_events() {
		let ext = load_test_ext(
			r#"
-- Use a module-level table to track calls.
local calls = {}
return function(api)
    api.on_agent_event(function(event)
        table.insert(calls, event.type)
    end)
    -- Expose calls via context so we can verify.
    api.set_context_provider("event_tracker")
end
"#,
		)
		.unwrap();

		let hooks = ext.hooks().expect("should have hooks");

		// Send a few events.
		hooks
			.on_agent_event(&rho_agent::events::AgentEvent::AgentStart)
			.await;
		hooks
			.on_agent_event(&rho_agent::events::AgentEvent::TurnStart { turn: 1 })
			.await;
		hooks
			.on_agent_event(&rho_agent::events::AgentEvent::TextDelta("hello".to_owned()))
			.await;

		// If it didn't panic, the handler was called successfully.
		// We can't easily read back the Lua `calls` table from Rust,
		// but absence of errors proves dispatch works.
		assert!(ext.context_provider().is_some());
	}

	// --- Tool concurrency ---

	#[test]
	fn tool_concurrency_exclusive() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "deploy",
        description = "Deploy tool",
        concurrency = "exclusive",
        input_schema = { type = "object" },
        execute = function(input, ctx)
            return { content = "deployed", is_error = false }
        end
    })
end
"#,
		)
		.unwrap();

		let tools = ext.tools();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].concurrency(), rho_agent::tools::Concurrency::Exclusive);
	}

	#[test]
	fn tool_concurrency_default_shared() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "search",
        description = "Search tool",
        input_schema = { type = "object" },
        execute = function(input, ctx)
            return { content = "found", is_error = false }
        end
    })
end
"#,
		)
		.unwrap();

		let tools = ext.tools();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].concurrency(), rho_agent::tools::Concurrency::Shared);
	}

	// --- Tool streaming on_update ---

	#[tokio::test]
	async fn tool_streaming_on_update() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "streamer",
        description = "Streams output",
        input_schema = { type = "object" },
        execute = function(input, ctx)
            if ctx.update then
                ctx.update("chunk1")
                ctx.update("chunk2")
            end
            return { content = "done", is_error = false }
        end
    })
end
"#,
		)
		.unwrap();

		let tools = ext.tools();
		let cancel = tokio_util::sync::CancellationToken::new();

		// Collect streaming updates.
		let chunks: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
			std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
		let chunks_clone = chunks.clone();
		let on_update: rho_agent::tools::OnToolUpdate =
			std::sync::Arc::new(move |text: &str| {
				chunks_clone.lock().unwrap().push(text.to_owned());
			});

		let result = tools[0]
			.execute(
				&serde_json::json!({}),
				Path::new("/tmp"),
				&cancel,
				Some(&on_update),
			)
			.await
			.unwrap();

		assert_eq!(result.content, "done");
		assert!(!result.is_error);

		let collected = chunks.lock().unwrap();
		assert_eq!(*collected, vec!["chunk1".to_owned(), "chunk2".to_owned()]);
	}

	// --- Extension metadata ---

	#[test]
	fn extension_id_and_name() {
		let ext = load_test_ext(
			r#"
return function(api)
end
"#,
		)
		.unwrap();
		assert_eq!(ext.id(), "test-ext");
		assert_eq!(ext.name(), "Test test-ext");
	}

	// --- Tool error handling: Lua error during execute ---

	#[tokio::test]
	async fn tool_execute_lua_error_returns_is_error() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "crasher",
        description = "Always crashes",
        input_schema = { type = "object" },
        execute = function(input, ctx)
            error("boom!")
        end
    })
end
"#,
		)
		.unwrap();

		let tools = ext.tools();
		let cancel = tokio_util::sync::CancellationToken::new();
		let result = tools[0]
			.execute(&serde_json::json!({}), Path::new("/tmp"), &cancel, None)
			.await
			.unwrap();
		assert!(result.is_error);
		assert!(result.content.contains("Extension error"));
	}

	// --- Multiple tools from one extension ---

	#[tokio::test]
	async fn multiple_tools_registered() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "tool_a",
        description = "First",
        input_schema = { type = "object" },
        execute = function(input, ctx)
            return { content = "a", is_error = false }
        end
    })
    api.register_tool({
        name = "tool_b",
        description = "Second",
        input_schema = { type = "object" },
        execute = function(input, ctx)
            return { content = "b", is_error = false }
        end
    })
end
"#,
		)
		.unwrap();

		let tools = ext.tools();
		assert_eq!(tools.len(), 2);
		assert_eq!(tools[0].name(), "tool_a");
		assert_eq!(tools[1].name(), "tool_b");
	}

	// --- Integration with ExtensionManager ---

	#[test]
	fn lua_extension_registers_in_manager() {
		let ext = load_test_ext(
			r#"
return function(api)
    api.register_tool({
        name = "lua_echo",
        description = "Echo from Lua",
        input_schema = { type = "object", properties = { msg = { type = "string" } } },
        execute = function(input, ctx)
            return { content = input.msg or "", is_error = false }
        end
    })
    api.set_context_provider("Lua extension active")
end
"#,
		)
		.unwrap();

		let mut mgr = crate::extensions::ExtensionManager::new();
		mgr.load(ext);

		let tools = mgr.extension_tools();
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].name(), "lua_echo");

		let ctx = mgr.context_strings();
		assert_eq!(ctx, vec!["Lua extension active"]);
	}
}
