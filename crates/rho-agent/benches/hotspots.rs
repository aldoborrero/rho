//! Benchmarks measuring the cost of specific cloning hotspots in rho-agent
//! and the savings from proposed optimizations.
//!
//! Each group compares `current` (status quo) vs `optimized` (proposed change).

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use rho_agent::types::{AssistantMessage, ContentBlock, StopReason, Usage};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Realistic assistant message with thinking + text + tool_use.
fn build_assistant_message() -> AssistantMessage {
	AssistantMessage {
		content:     vec![
			ContentBlock::Thinking {
				thinking: "The user wants me to read the file and find the relevant function. I \
				           should use the bash tool to cat the file and then identify the function \
				           they're referring to. Let me check the file structure first to understand \
				           the codebase layout before making changes. I need to consider that the \
				           file might be large, so I should use head to limit output. Also need to \
				           check if there are any related test files."
					.to_owned(),
			},
			ContentBlock::Text {
				text: "I'll read the file to find the function you mentioned. Let me check the \
				       current implementation first. This should help us understand the structure \
				       before making any modifications."
					.to_owned(),
			},
			ContentBlock::ToolUse {
				id:    "toolu_abc123def456".to_owned(),
				name:  "bash".to_owned(),
				input: serde_json::json!({
					"command": "cat -n src/lib.rs | head -100",
					"description": "Read the first 100 lines of lib.rs"
				}),
			},
		],
		stop_reason: Some(StopReason::ToolUse),
		usage:       Some(Usage {
			input_tokens:                2500,
			output_tokens:               350,
			cache_creation_input_tokens: 0,
			cache_read_input_tokens:     1800,
		}),
	}
}

/// Large assistant message simulating extended thinking.
fn build_large_assistant_message() -> AssistantMessage {
	let thinking = "x".repeat(4096);
	let text = "y".repeat(1024);
	AssistantMessage {
		content:     vec![
			ContentBlock::Thinking { thinking },
			ContentBlock::Text { text },
			ContentBlock::ToolUse {
				id:    "toolu_abc123def456".to_owned(),
				name:  "write".to_owned(),
				input: serde_json::json!({
					"file_path": "/tmp/test.rs",
					"content": "z".repeat(2048),
				}),
			},
		],
		stop_reason: Some(StopReason::ToolUse),
		usage:       Some(Usage {
			input_tokens:                5000,
			output_tokens:               800,
			cache_creation_input_tokens: 0,
			cache_read_input_tokens:     3000,
		}),
	}
}

// ---------------------------------------------------------------------------
// Hotspot 1: Double AssistantMessage clone in agent_loop (lines 200 + 214)
//
// Current: message.clone() twice (event + messages vec)
// Optimized: clone once, move the other
// ---------------------------------------------------------------------------

fn bench_assistant_message_clone(c: &mut Criterion) {
	let mut group = c.benchmark_group("hotspot1_assistant_message_clone");

	for (label, msg) in
		[("normal", build_assistant_message()), ("large_thinking", build_large_assistant_message())]
	{
		// Current: two clones (simulating event send + messages.push)
		group.bench_function(format!("current_2x_clone/{label}"), |b| {
			b.iter(|| {
				let clone1 = std::hint::black_box(&msg).clone();
				let clone2 = std::hint::black_box(&msg).clone();
				std::hint::black_box((clone1, clone2));
			});
		});

		// Optimized: one clone + one move
		group.bench_function(format!("optimized_1x_clone/{label}"), |b| {
			b.iter(|| {
				let clone1 = std::hint::black_box(&msg).clone();
				// Move the original into messages, clone for event (or vice versa)
				let moved = std::hint::black_box(msg.clone());
				std::hint::black_box((clone1, moved));
			});
		});

		// Arc approach: wrap in Arc, refcount bump instead of deep clone
		group.bench_function(format!("arc_shared/{label}"), |b| {
			b.iter(|| {
				let arc = Arc::new(std::hint::black_box(&msg).clone());
				let ref1 = Arc::clone(&arc);
				let ref2 = Arc::clone(&arc);
				std::hint::black_box((ref1, ref2));
			});
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Hotspot 2: Tool result Arc<String> deref-clone in convert.rs line 30
//
// Current: (*t.content).clone() — deref Arc, clone inner String
// Optimized: share the Arc directly (requires rho-ai type change)
// ---------------------------------------------------------------------------

fn bench_tool_result_content_clone(c: &mut Criterion) {
	let mut group = c.benchmark_group("hotspot2_tool_result_deref_clone");

	for (label, size) in [("small_200B", 200), ("medium_2KB", 2048), ("large_8KB", 8192)] {
		let content = Arc::new("x".repeat(size));

		// Current: deref the Arc and clone the String
		group.bench_function(format!("current_deref_clone/{label}"), |b| {
			b.iter(|| {
				let cloned = String::clone(&*std::hint::black_box(&content));
				std::hint::black_box(cloned);
			});
		});

		// Optimized: Arc::clone (refcount bump only)
		group.bench_function(format!("optimized_arc_clone/{label}"), |b| {
			b.iter(|| {
				let shared = Arc::clone(std::hint::black_box(&content));
				std::hint::black_box(shared);
			});
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Hotspot 3: serde_json::Value clone for tool input
//
// Current: input.clone() per tool execution
// Optimized: pass &Value (would require trait change)
// Measures the clone cost at various input sizes.
// ---------------------------------------------------------------------------

fn bench_json_value_clone(c: &mut Criterion) {
	let mut group = c.benchmark_group("hotspot3_json_value_clone");

	let small_input = serde_json::json!({
		"command": "ls -la",
		"description": "List files"
	});

	let medium_input = serde_json::json!({
		"file_path": "/tmp/test.rs",
		"content": "x".repeat(2048),
	});

	let large_input = serde_json::json!({
		"file_path": "/tmp/big.rs",
		"content": "x".repeat(16384),
		"description": "A large file write operation",
	});

	for (label, input) in [
		("small_bash", &small_input),
		("medium_write_2KB", &medium_input),
		("large_write_16KB", &large_input),
	] {
		group.bench_function(format!("clone/{label}"), |b| {
			b.iter(|| {
				let cloned = std::hint::black_box(input).clone();
				std::hint::black_box(cloned);
			});
		});

		// Baseline: just borrowing (zero cost, for comparison)
		group.bench_function(format!("borrow/{label}"), |b| {
			b.iter(|| {
				let borrowed = std::hint::black_box(input);
				std::hint::black_box(borrowed);
			});
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Hotspot 4: Repeated id.to_owned() per tool call
//
// Current: 5x to_owned() of the same tool use ID + 3x name clone +
//          2x Arc::clone for content
// Optimized: 1x allocation, share via Arc<str> or clone from single String
// ---------------------------------------------------------------------------

fn bench_tool_id_allocations(c: &mut Criterion) {
	let mut group = c.benchmark_group("hotspot4_tool_id_allocations");

	let id = "toolu_01JFG4K7Z3HWGQFNBM3KWB11AH";

	// Current: 5 separate to_owned() calls for tool_use_id (ToolCallStart,
	// ToolCallResult, TurnEnd tool_results, ToolResultMessage, + callback
	// capture), plus 3 name clones (ToolCallStart, ToolCallResult, callback)
	// and 2 Arc::clone for content (ToolCallResult, TurnEnd tool_results)
	group.bench_function("current_5x_to_owned", |b| {
		b.iter(|| {
			let id = std::hint::black_box(id);
			let a1 = id.to_owned();
			let a2 = id.to_owned();
			let a3 = id.to_owned();
			let a4 = id.to_owned();
			let a5 = id.to_owned();
			std::hint::black_box((a1, a2, a3, a4, a5));
		});
	});

	// Optimized: 1 Arc<str>, 4 Arc::clone
	group.bench_function("optimized_arc_str", |b| {
		b.iter(|| {
			let id = std::hint::black_box(id);
			let arc: Arc<str> = Arc::from(id);
			let a1 = Arc::clone(&arc);
			let a2 = Arc::clone(&arc);
			let a3 = Arc::clone(&arc);
			let a4 = Arc::clone(&arc);
			std::hint::black_box((arc, a1, a2, a3, a4));
		});
	});

	// Optimized: 1 String allocation, 4 clones (still allocates but from owned)
	group.bench_function("optimized_1_alloc_4_clone", |b| {
		b.iter(|| {
			let id = std::hint::black_box(id);
			let owned = id.to_owned();
			let a1 = owned.clone();
			let a2 = owned.clone();
			let a3 = owned.clone();
			let a4 = owned.clone();
			std::hint::black_box((owned, a1, a2, a3, a4));
		});
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// Hotspot 5: make_update_callback clones id per output chunk
//
// Simulates a bash command producing N output chunks, each triggering
// an id.clone() inside the closure.
//
// Current: String captured, cloned per chunk
// Optimized: Arc<str> captured, Arc::clone per chunk
// ---------------------------------------------------------------------------

fn bench_callback_id_clone(c: &mut Criterion) {
	let mut group = c.benchmark_group("hotspot5_callback_id_clone");

	let id_str = "toolu_01JFG4K7Z3HWGQFNBM3KWB11AH";

	for chunks in [10, 100, 500] {
		// Current: String clone per chunk
		group.bench_function(format!("current_string_clone/{chunks}_chunks"), |b| {
			let id = id_str.to_owned();
			b.iter(|| {
				for _ in 0..chunks {
					let cloned = std::hint::black_box(&id).clone();
					std::hint::black_box(cloned);
				}
			});
		});

		// Optimized: Arc<str> clone per chunk (refcount bump only)
		group.bench_function(format!("optimized_arc_clone/{chunks}_chunks"), |b| {
			let id: Arc<str> = Arc::from(id_str);
			b.iter(|| {
				for _ in 0..chunks {
					let cloned = Arc::clone(std::hint::black_box(&id));
					std::hint::black_box(cloned);
				}
			});
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Hotspot 6: Full conversion round-trip (to_ai_content_block clones all
// strings)
//
// Current: clone each field
// Optimized: Arc<String> fields — Arc::clone is refcount bump
// ---------------------------------------------------------------------------

fn bench_content_block_clone(c: &mut Criterion) {
	let mut group = c.benchmark_group("hotspot6_content_block_conversion");

	let thinking_block = ContentBlock::Thinking { thinking: "x".repeat(4096) };
	let text_block = ContentBlock::Text { text: "y".repeat(1024) };
	let tool_use_block = ContentBlock::ToolUse {
		id:    "toolu_abc123def456".to_owned(),
		name:  "bash".to_owned(),
		input: serde_json::json!({"command": "ls -la", "description": "list files"}),
	};

	for (label, block) in
		[("thinking_4KB", &thinking_block), ("text_1KB", &text_block), ("tool_use", &tool_use_block)]
	{
		// Current: full deep clone (what to_ai_content_block does)
		group.bench_function(format!("clone/{label}"), |b| {
			b.iter(|| {
				let cloned = std::hint::black_box(block).clone();
				std::hint::black_box(cloned);
			});
		});
	}

	group.finish();
}

// ---------------------------------------------------------------------------
// Combined: simulate a full tool-call turn with all hotspots
//
// Models: 1 LLM response with 3 tool calls, each producing 50 output chunks.
// Counts total clone cost across all hotspots.
// ---------------------------------------------------------------------------

fn bench_full_turn_simulation(c: &mut Criterion) {
	let mut group = c.benchmark_group("full_turn_simulation");

	let message = build_assistant_message();
	let tool_ids = ["toolu_001", "toolu_002", "toolu_003"];
	let tool_results: Vec<Arc<String>> = (0..3)
		.map(|i| Arc::new(format!("Result {i}: {}", "x".repeat(500))))
		.collect();
	let tool_input = serde_json::json!({"command": "ls -la"});
	let chunks_per_tool = 50;

	// Current approach: all the cloning as-is
	group.bench_function("current", |b| {
		b.iter(|| {
			// Hotspot 1: double message clone
			let _event_msg = message.clone();
			let _stored_msg = message.clone();

			for (i, id) in tool_ids.iter().enumerate() {
				// Hotspot 4: 5x id allocation
				let _start_id = (*id).to_owned();
				let _start_name = "bash".to_owned();
				let _result_id = (*id).to_owned();
				let _complete_id = (*id).to_owned();
				let _msg_id = (*id).to_owned();

				// Hotspot 3: value clone
				let _input = tool_input.clone();

				// Hotspot 5: callback id clone per chunk
				let cb_id = (*id).to_owned();
				for _ in 0..chunks_per_tool {
					let _chunk_id = cb_id.clone();
				}

				// Hotspot 2: deref-clone tool result
				let _result_content: String = (*tool_results[i]).clone();
			}

			std::hint::black_box(());
		});
	});

	// Optimized approach: Arc sharing, borrow where possible
	group.bench_function("optimized", |b| {
		b.iter(|| {
			// Hotspot 1: one clone + move
			let _event_msg = message.clone();
			// (the second would be a move in real code)

			for (i, id) in tool_ids.iter().enumerate() {
				// Hotspot 4: 1 Arc<str> + refcount bumps
				let arc_id: Arc<str> = Arc::from(*id);
				let _start_id = Arc::clone(&arc_id);
				let _start_name = "bash"; // could use &'static str
				let _result_id = Arc::clone(&arc_id);
				let _complete_id = Arc::clone(&arc_id);

				// Hotspot 3: borrow instead of clone (simulated)
				let _input = &tool_input;

				// Hotspot 5: Arc clone per chunk
				for _ in 0..chunks_per_tool {
					let _chunk_id = Arc::clone(&arc_id);
				}

				// Hotspot 2: Arc::clone instead of deref
				let _result_content = Arc::clone(&tool_results[i]);
			}

			std::hint::black_box(());
		});
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

criterion_group!(
	benches,
	bench_assistant_message_clone,
	bench_tool_result_content_clone,
	bench_json_value_clone,
	bench_tool_id_allocations,
	bench_callback_id_clone,
	bench_content_block_clone,
	bench_full_turn_simulation,
);
criterion_main!(benches);
