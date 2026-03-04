//! Compaction orchestrator: summarize older messages via LLM.
//!
//! oh-my-pi ref: `compaction.ts` `compact()` lines 732-825,
//! `generateSummary()` lines 486-580

use anyhow::{Result, bail};

use super::{
	cut_point::find_cut_point,
	file_ops::{compute_file_lists, extract_file_ops},
	serialize::serialize_conversation,
	settings::{CompactionSettings, effective_reserve_tokens},
	tokens::estimate_entry_tokens,
};
use crate::{
	ai::types::Message,
	prompts::compaction::{
		SHORT_SUMMARY_PROMPT, SUMMARIZATION_SYSTEM, SUMMARY_PROMPT, UPDATE_SUMMARY_PROMPT,
		render_file_operations,
	},
	session::{SessionManager, types::SessionEntry},
};

/// Result of a compaction operation.
pub struct CompactionResult {
	/// The full structured summary.
	pub summary:             String,
	/// Short PR-style summary (2-3 sentences).
	pub short_summary:       Option<String>,
	/// ID of the first session entry that was kept (not summarized).
	pub first_kept_entry_id: String,
	/// Estimated total tokens before compaction.
	pub tokens_before:       u64,
	/// File operation details (`readFiles`, `modifiedFiles`).
	pub details:             Option<serde_json::Value>,
}

/// Main compaction orchestrator.
///
/// 1. Gets the current branch entries.
/// 2. Finds the cut point (what to keep vs summarize).
/// 3. Calls the LLM to generate a summary.
/// 4. Returns a [`CompactionResult`] for the caller to persist.
///
/// oh-my-pi ref: `compaction.ts` `compact()` lines 732-825
#[allow(
	clippy::future_not_send,
	reason = "SessionManager is !Sync; compaction runs on the main task"
)]
pub async fn run_compaction(
	session: &SessionManager,
	model: &rho_ai::Model,
	api_key: &str,
	settings: &CompactionSettings,
	custom_instructions: Option<&str>,
) -> Result<CompactionResult> {
	// Get the current branch entries in root-to-leaf order.
	let branch: Vec<&SessionEntry> = match session.leaf_id() {
		Some(id) => {
			let mut b = session.get_branch(id);
			b.reverse();
			b
		},
		None => bail!("No entries to compact"),
	};

	if branch.is_empty() {
		bail!("No entries to compact");
	}

	// Find the latest compaction entry to determine our start point.
	let (start_idx, previous_summary) = branch
		.iter()
		.enumerate()
		.rev()
		.find_map(|(i, entry)| {
			if let SessionEntry::Compaction(c) = entry {
				Some((i + 1, Some(c.summary.clone())))
			} else {
				None
			}
		})
		.unwrap_or((0, None));

	let end = branch.len();
	if start_idx >= end {
		bail!("Nothing to compact after previous compaction");
	}

	// Find the cut point.
	let cut = find_cut_point(&branch, start_idx, end, settings.keep_recent_tokens)
		.ok_or_else(|| anyhow::anyhow!("Not enough content to compact"))?;

	if cut.first_kept_index <= start_idx {
		bail!("Not enough content to compact (cut point at start)");
	}

	// Extract messages to summarize (between start and cut point).
	let messages_to_summarize: Vec<Message> = branch[start_idx..cut.first_kept_index]
		.iter()
		.filter_map(|entry| {
			if let SessionEntry::Message(m) = entry {
				Some(m.message.clone())
			} else {
				None
			}
		})
		.collect();

	if messages_to_summarize.is_empty() {
		bail!("No messages to summarize");
	}

	// Estimate total tokens before compaction.
	let tokens_before: u64 = branch[start_idx..end]
		.iter()
		.map(|e| u64::from(estimate_entry_tokens(e)))
		.sum();

	let reserve = effective_reserve_tokens(model.context_window, settings);

	// Generate the summary.
	let summary = generate_summary(
		model,
		&messages_to_summarize,
		reserve,
		api_key,
		custom_instructions,
		previous_summary.as_deref(),
	)
	.await?;

	// Generate short summary (best-effort).
	let short_summary = generate_short_summary(model, &messages_to_summarize, reserve, api_key)
		.await
		.ok();

	// Get the first kept entry ID.
	let first_kept_entry_id = branch[cut.first_kept_index].id().to_owned();

	// Compute file operation details.
	let file_ops = extract_file_ops(&messages_to_summarize);
	let (read_files, modified_files) = compute_file_lists(&file_ops);
	let details = if read_files.is_empty() && modified_files.is_empty() {
		None
	} else {
		Some(serde_json::json!({
			"readFiles": read_files,
			"modifiedFiles": modified_files,
		}))
	};

	Ok(CompactionResult { summary, short_summary, first_kept_entry_id, tokens_before, details })
}

/// Generate a summary of messages using the LLM.
///
/// Constructs a `rho_ai::Context` with the summarization system prompt,
/// serialized conversation, and the appropriate summary prompt. If a
/// `previous_summary` exists, uses the iterative update prompt.
///
/// oh-my-pi ref: `compaction.ts` `generateSummary()` lines 486-580
async fn generate_summary(
	model: &rho_ai::Model,
	messages_to_summarize: &[Message],
	reserve_tokens: u32,
	api_key: &str,
	custom_instructions: Option<&str>,
	previous_summary: Option<&str>,
) -> Result<String> {
	let serialized = serialize_conversation(messages_to_summarize);

	// Build the user message content.
	let mut user_content = format!("<conversation>\n{serialized}</conversation>\n\n");

	// Add file operations context.
	let file_ops = extract_file_ops(messages_to_summarize);
	let (read_files, modified_files) = compute_file_lists(&file_ops);
	let file_ops_text = render_file_operations(&read_files, &modified_files);
	if !file_ops_text.is_empty() {
		user_content.push_str(&file_ops_text);
		user_content.push('\n');
	}

	// Choose prompt based on whether we have a previous summary.
	if let Some(prev) = previous_summary {
		user_content.push_str("<previous-summary>\n");
		user_content.push_str(prev);
		user_content.push_str("\n</previous-summary>\n\n");
		user_content.push_str(UPDATE_SUMMARY_PROMPT);
	} else {
		user_content.push_str(SUMMARY_PROMPT);
	}

	// Add custom instructions if provided.
	if let Some(instructions) = custom_instructions {
		user_content.push_str("\n\nAdditional instructions: ");
		user_content.push_str(instructions);
	}

	call_summarization_llm(model, &user_content, reserve_tokens, api_key).await
}

/// Generate a short PR-style summary (2-3 sentences).
///
/// oh-my-pi ref: `compaction.ts` `generateShortSummary()`
async fn generate_short_summary(
	model: &rho_ai::Model,
	messages: &[Message],
	reserve_tokens: u32,
	api_key: &str,
) -> Result<String> {
	let serialized = serialize_conversation(messages);
	let user_content =
		format!("<conversation>\n{serialized}</conversation>\n\n{SHORT_SUMMARY_PROMPT}");

	call_summarization_llm(model, &user_content, reserve_tokens.min(1024), api_key).await
}

/// Call the LLM with the summarization system prompt and user content.
async fn call_summarization_llm(
	model: &rho_ai::Model,
	user_content: &str,
	max_tokens: u32,
	api_key: &str,
) -> Result<String> {
	// Construct the rho_ai::Context (different type system from session messages).
	let context = rho_ai::Context {
		system_prompt: Some(std::sync::Arc::new(SUMMARIZATION_SYSTEM.to_owned())),
		messages:      vec![rho_ai::types::Message::User(rho_ai::types::UserMessage {
			content: vec![rho_ai::types::UserContent::Text { text: user_content.to_owned() }],
		})],
		tools:         vec![],
	};

	#[allow(
		clippy::cast_possible_truncation,
		clippy::cast_sign_loss,
		reason = "80% of reserve tokens always fits in u32"
	)]
	let effective_max = ((f64::from(max_tokens)) * 0.8) as u32;

	let options = rho_ai::StreamOptions {
		api_key: Some(api_key.to_owned()),
		max_tokens: Some(effective_max),
		..Default::default()
	};

	let response = rho_ai::complete(model, &context, &options)
		.await
		.map_err(|e| anyhow::anyhow!("LLM call failed: {e}"))?;

	// Extract text from response content blocks.
	let text: String = response
		.content
		.iter()
		.filter_map(|block| {
			if let rho_ai::types::ContentBlock::Text { text } = block {
				Some(text.as_str())
			} else {
				None
			}
		})
		.collect::<Vec<_>>()
		.join("");

	if text.is_empty() {
		bail!("Summarization returned empty response");
	}

	Ok(text)
}
