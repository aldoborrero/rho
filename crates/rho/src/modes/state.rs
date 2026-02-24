//! Application mode state machine.

/// Application mode — only one active at a time.
/// Prevents impossible states (e.g., compacting while streaming).
pub enum AppMode {
	/// Waiting for user input.
	Idle,
	/// Agent is running, streaming tokens.
	Streaming,
	/// A bang command (`!cmd`) is running with streaming output.
	BangRunning,
	/// Compaction LLM call in progress.
	#[allow(dead_code, reason = "variant reserved for upcoming compaction feature")]
	Compacting,
	/// Plan mode — agent writes to plan file, user approves.
	#[allow(dead_code, reason = "variant reserved for upcoming plan mode feature")]
	PlanMode { file_path: String },
	/// Interactive selector UI is open.
	#[allow(dead_code, reason = "variant reserved for upcoming selector UI")]
	Selecting(SelectorKind),
}

#[allow(dead_code, reason = "variants reserved for upcoming selector UI")]
pub enum SelectorKind {
	SessionPicker,
	BranchTree,
	ModelPicker,
	Settings,
}
