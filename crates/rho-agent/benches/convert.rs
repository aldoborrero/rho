use criterion::{Criterion, criterion_group, criterion_main};
use rho_agent::types::{
	AssistantMessage, ContentBlock, Message, StopReason, ToolDefinition, ToolResultMessage, Usage,
	UserMessage,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Build a realistic conversation of `turns` agent-loop iterations.
///
/// Each turn produces 3 messages:
///   1. Assistant — 1 text block + 1 tool_use block (realistic JSON input)
///   2. Tool result — ~200 chars of output
///   3. User follow-up
///
/// So `build_conversation(100)` yields ~300 messages.
fn build_conversation(turns: usize) -> Vec<Message> {
	let mut msgs = Vec::with_capacity(turns * 3);
	for i in 0..turns {
		let id = format!("toolu_{i:06}");
		msgs.push(Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::Text {
					text: format!("Let me check that file for you (turn {i})."),
				},
				ContentBlock::ToolUse {
					id:    id.clone(),
					name:  "bash".to_owned(),
					input: serde_json::json!({
						"command": format!("cat src/main.rs | head -n 50"),
						"description": "Read the first 50 lines of main.rs",
						"timeout": 30000
					}),
				},
			],
			stop_reason: Some(StopReason::ToolUse),
			usage:       Some(Usage {
				input_tokens:                1200 + (i as u32 * 100),
				output_tokens:               150,
				cache_creation_input_tokens: 0,
				cache_read_input_tokens:     800,
			}),
		}));
		msgs.push(Message::ToolResult(ToolResultMessage {
			tool_use_id: id,
			content:     std::sync::Arc::new(format!(
				"use std::io::{{self, Write}};\n\
				 fn main() -> io::Result<()> {{\n\
				     let mut stdout = io::stdout().lock();\n\
				     writeln!(stdout, \"Hello from turn {i}\")?;\n\
				     Ok(())\n\
				 }}\n\
				 // end of file (line ~200 chars padding: {})",
				"x".repeat(80)
			)),
			is_error:    false,
		}));
		msgs.push(Message::User(UserMessage {
			content: format!("Now update the function on line 3 to accept a name parameter (turn {i})."),
		}));
	}
	msgs
}

/// Build realistic tool definitions matching the 11 built-in tools.
fn build_tool_defs(count: usize) -> Vec<ToolDefinition> {
	let templates = [
		(
			"bash",
			"Execute a bash command in a sandboxed environment",
			serde_json::json!({
				"type": "object",
				"properties": {
					"command": {"type": "string", "description": "The bash command to execute"},
					"description": {"type": "string", "description": "Short description of the command"},
					"timeout": {"type": "number", "description": "Timeout in milliseconds"}
				},
				"required": ["command"]
			}),
		),
		(
			"read",
			"Read a file from the filesystem with optional line range",
			serde_json::json!({
				"type": "object",
				"properties": {
					"file_path": {"type": "string", "description": "Absolute path to the file"},
					"offset": {"type": "number", "description": "Starting line number"},
					"limit": {"type": "number", "description": "Number of lines to read"}
				},
				"required": ["file_path"]
			}),
		),
		(
			"write",
			"Write content to a file, creating it if necessary",
			serde_json::json!({
				"type": "object",
				"properties": {
					"file_path": {"type": "string", "description": "Absolute path to the file"},
					"content": {"type": "string", "description": "Content to write"}
				},
				"required": ["file_path", "content"]
			}),
		),
		(
			"grep",
			"Search file contents using regular expressions",
			serde_json::json!({
				"type": "object",
				"properties": {
					"pattern": {"type": "string", "description": "Regex pattern to search for"},
					"path": {"type": "string", "description": "Directory or file to search in"},
					"include": {"type": "string", "description": "Glob pattern for files to include"}
				},
				"required": ["pattern"]
			}),
		),
		(
			"find",
			"Find files matching a glob pattern",
			serde_json::json!({
				"type": "object",
				"properties": {
					"pattern": {"type": "string", "description": "Glob pattern to match"},
					"path": {"type": "string", "description": "Directory to search in"}
				},
				"required": ["pattern"]
			}),
		),
		(
			"fuzzy_find",
			"Fuzzy find files by name",
			serde_json::json!({
				"type": "object",
				"properties": {
					"query": {"type": "string", "description": "Fuzzy search query"},
					"path": {"type": "string", "description": "Directory to search in"}
				},
				"required": ["query"]
			}),
		),
		(
			"clipboard",
			"Read from or write to the system clipboard",
			serde_json::json!({
				"type": "object",
				"properties": {
					"action": {"type": "string", "enum": ["read", "write"]},
					"content": {"type": "string", "description": "Content to write (write only)"}
				},
				"required": ["action"]
			}),
		),
		(
			"html_to_markdown",
			"Convert HTML content to Markdown format",
			serde_json::json!({
				"type": "object",
				"properties": {
					"html": {"type": "string", "description": "HTML content to convert"}
				},
				"required": ["html"]
			}),
		),
		(
			"process",
			"Manage system processes (list, kill)",
			serde_json::json!({
				"type": "object",
				"properties": {
					"action": {"type": "string", "enum": ["list", "kill"]},
					"pid": {"type": "number", "description": "Process ID (kill only)"},
					"signal": {"type": "string", "description": "Signal to send (kill only)"}
				},
				"required": ["action"]
			}),
		),
		(
			"image",
			"Process and analyze image files",
			serde_json::json!({
				"type": "object",
				"properties": {
					"file_path": {"type": "string", "description": "Path to the image file"},
					"action": {"type": "string", "enum": ["read", "resize", "info"]}
				},
				"required": ["file_path"]
			}),
		),
		(
			"workmux",
			"Manage workmux sessions and panes",
			serde_json::json!({
				"type": "object",
				"properties": {
					"action": {"type": "string", "enum": ["create", "list", "send", "read"]},
					"session": {"type": "string", "description": "Session name"},
					"command": {"type": "string", "description": "Command to send"}
				},
				"required": ["action"]
			}),
		),
	];

	templates
		.iter()
		.cycle()
		.take(count)
		.map(|(name, desc, schema)| ToolDefinition {
			name:         (*name).to_owned(),
			description:  (*desc).to_owned(),
			input_schema: schema.clone(),
		})
		.collect()
}

/// Build a realistic AI assistant message for `from_ai_assistant` benchmarks.
fn build_ai_assistant_message() -> rho_ai::types::AssistantMessage {
	rho_ai::types::AssistantMessage {
		content:     vec![
			rho_ai::types::ContentBlock::Thinking {
				thinking: "The user wants me to read the file and find the relevant function. \
				           I should use the bash tool to cat the file and then identify the \
				           function they're referring to. Let me check the file structure first \
				           to understand the codebase layout before making changes."
					.to_owned(),
			},
			rho_ai::types::ContentBlock::Text {
				text: "I'll read the file to find the function you mentioned. Let me check \
				       the current implementation first."
					.to_owned(),
			},
			rho_ai::types::ContentBlock::ToolUse {
				id:    "toolu_abc123def456".to_owned(),
				name:  "bash".to_owned(),
				input: serde_json::json!({
					"command": "cat -n src/lib.rs | head -100",
					"description": "Read the first 100 lines of lib.rs"
				}),
			},
		],
		stop_reason: Some(rho_ai::types::StopReason::ToolUse),
		usage:       Some(rho_ai::types::Usage {
			input_tokens:       2500,
			output_tokens:      350,
			cache_read_tokens:  1800,
			cache_write_tokens: 0,
		}),
	}
}

// ---------------------------------------------------------------------------
// Group 1: to_ai_messages (full reconversion — the old hot path)
// ---------------------------------------------------------------------------

fn bench_to_ai_messages(c: &mut Criterion) {
	let mut group = c.benchmark_group("to_ai_messages");

	for turns in [10, 50, 100, 500] {
		let conversation = build_conversation(turns);
		group.bench_function(format!("{turns}_turns"), |b| {
			b.iter(|| rho_agent::convert::to_ai_messages(std::hint::black_box(&conversation)));
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Group 2: push_ai_message (incremental — the new hot path)
// ---------------------------------------------------------------------------

fn bench_push_ai_message(c: &mut Criterion) {
	let mut group = c.benchmark_group("push_ai_message");

	// The single message we'll append each iteration.
	let new_msg = Message::Assistant(AssistantMessage {
		content:     vec![
			ContentBlock::Text { text: "Here is the updated code.".to_owned() },
			ContentBlock::ToolUse {
				id:    "toolu_new".to_owned(),
				name:  "write".to_owned(),
				input: serde_json::json!({
					"file_path": "/tmp/test.rs",
					"content": "fn main() {}\n"
				}),
			},
		],
		stop_reason: Some(StopReason::ToolUse),
		usage:       None,
	});

	for turns in [10, 50, 100, 500] {
		let conversation = build_conversation(turns);
		// Pre-populate the AI message vec with the full history + spare capacity.
		let mut ai_messages = rho_agent::convert::to_ai_messages(&conversation);
		ai_messages.reserve(1);

		group.bench_function(format!("{turns}_turns"), |b| {
			// Use iter_batched_ref so the vec's drop cost is excluded from timing.
			b.iter_batched_ref(
				|| ai_messages.clone(),
				|dest| {
					rho_agent::convert::push_ai_message(
						std::hint::black_box(dest),
						std::hint::black_box(&new_msg),
					);
					// Pop the message so the vec size stays stable across iterations
					// within the same batch (SmallInput may run multiple iters per setup).
					dest.pop();
				},
				criterion::BatchSize::SmallInput,
			);
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Group 3: to_ai_tool_defs (tool definition conversion)
// ---------------------------------------------------------------------------

fn bench_to_ai_tool_defs(c: &mut Criterion) {
	let mut group = c.benchmark_group("to_ai_tool_defs");

	let defs = build_tool_defs(11);
	group.bench_function("11_tools", |b| {
		b.iter(|| rho_agent::convert::to_ai_tool_defs(std::hint::black_box(&defs)));
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// Group 4: from_ai_assistant (ai → agent conversion)
// ---------------------------------------------------------------------------

fn bench_from_ai_assistant(c: &mut Criterion) {
	let mut group = c.benchmark_group("from_ai_assistant");

	let ai_msg = build_ai_assistant_message();
	group.bench_function("thinking_text_tool_use", |b| {
		b.iter(|| rho_agent::convert::from_ai_assistant(std::hint::black_box(&ai_msg)));
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// Group 5: context_construction (swap vs clone)
// ---------------------------------------------------------------------------

fn bench_context_construction(c: &mut Criterion) {
	let mut group = c.benchmark_group("context_construction");

	for turns in [10, 50, 100, 500] {
		let conversation = build_conversation(turns);
		let ai_messages = rho_agent::convert::to_ai_messages(&conversation);
		let tool_defs = build_tool_defs(11);
		let ai_tools = rho_agent::convert::to_ai_tool_defs(&tool_defs);

		// clone: the naive approach — clone the full vec into Context.
		group.bench_function(format!("clone/{turns}_turns"), |b| {
			b.iter(|| {
				let ctx = rho_ai::types::Context {
					system_prompt: Some(std::sync::Arc::new("You are a helpful assistant.".to_owned())),
					messages:      std::hint::black_box(&ai_messages).clone(),
					tools:         std::hint::black_box(&ai_tools).clone(),
				};
				std::hint::black_box(ctx);
			});
		});

		// swap: our approach — std::mem::take + reclaim.
		group.bench_function(format!("swap/{turns}_turns"), |b| {
			b.iter_batched(
				|| (ai_messages.clone(), ai_tools.clone()),
				|(mut msgs, mut tools)| {
					let mut ctx = rho_ai::types::Context {
						system_prompt: Some(std::sync::Arc::new("You are a helpful assistant.".to_owned())),
						messages:      std::mem::take(&mut msgs),
						tools:         std::mem::take(&mut tools),
					};
					// Reclaim after "streaming" (simulates the real agent loop pattern).
					let _msgs = std::mem::take(&mut ctx.messages);
					let _tools = std::mem::take(&mut ctx.tools);
					std::hint::black_box((_msgs, _tools));
				},
				criterion::BatchSize::SmallInput,
			);
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

criterion_group!(
	benches,
	bench_to_ai_messages,
	bench_push_ai_message,
	bench_to_ai_tool_defs,
	bench_from_ai_assistant,
	bench_context_construction,
);
criterion_main!(benches);
