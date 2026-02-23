//! Terminal multiplexer + git worktree orchestration via workmux.
//!
//! # Overview
//! Provides pure Rust bindings for workmux library functionality, enabling
//! oh-my-pi swarm agents to:
//! - Run in isolated git worktrees with dedicated terminal windows
//! - Report status to the workmux dashboard
//! - Coordinate merge workflows

use workmux::{
	AgentStatus, BackendType, CreateWindowParams, StateStore, create_backend, detect_backend,
	persist_agent_update,
};

use crate::error::{Error, Result};

/// Detected terminal multiplexer backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkmuxBackend {
	Tmux,
	WezTerm,
	Kitty,
}

impl From<BackendType> for WorkmuxBackend {
	fn from(bt: BackendType) -> Self {
		match bt {
			BackendType::Tmux => Self::Tmux,
			BackendType::WezTerm => Self::WezTerm,
			BackendType::Kitty => Self::Kitty,
		}
	}
}

impl From<WorkmuxBackend> for BackendType {
	fn from(wb: WorkmuxBackend) -> Self {
		match wb {
			WorkmuxBackend::Tmux => Self::Tmux,
			WorkmuxBackend::WezTerm => Self::WezTerm,
			WorkmuxBackend::Kitty => Self::Kitty,
		}
	}
}

/// Agent status for workmux dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkmuxAgentStatus {
	Working,
	Waiting,
	Done,
}

impl From<WorkmuxAgentStatus> for AgentStatus {
	fn from(status: WorkmuxAgentStatus) -> Self {
		match status {
			WorkmuxAgentStatus::Working => Self::Working,
			WorkmuxAgentStatus::Waiting => Self::Waiting,
			WorkmuxAgentStatus::Done => Self::Done,
		}
	}
}

impl From<AgentStatus> for WorkmuxAgentStatus {
	fn from(status: AgentStatus) -> Self {
		match status {
			AgentStatus::Working => Self::Working,
			AgentStatus::Waiting => Self::Waiting,
			AgentStatus::Done => Self::Done,
		}
	}
}

/// Information about the current workmux environment.
pub struct WorkmuxEnvironment {
	/// Detected backend type.
	pub backend:    WorkmuxBackend,
	/// Whether the multiplexer server is running.
	pub is_running: bool,
	/// Current pane ID if inside a multiplexer pane.
	pub pane_id:    Option<String>,
}

/// Detect the terminal multiplexer backend and check if it's running.
///
/// # Returns
/// Environment info including backend type, running status, and current pane
/// ID.
pub fn workmux_detect_environment() -> Result<WorkmuxEnvironment> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	let is_running = mux.is_running().unwrap_or(false);
	let pane_id = mux.current_pane_id();

	Ok(WorkmuxEnvironment { backend: backend_type.into(), is_running, pane_id })
}

/// Parameters for creating a new multiplexer window.
pub struct WorkmuxCreateWindowParams {
	/// Window name prefix (e.g., "swarm-").
	pub prefix:       String,
	/// Window name (without prefix).
	pub name:         String,
	/// Working directory for the window.
	pub cwd:          String,
	/// Optional window ID to insert after (for ordering).
	pub after_window: Option<String>,
}

/// Create a new multiplexer window/tab.
///
/// # Parameters
/// - `params`: Window creation parameters (prefix, name, cwd, optional
///   `after_window`)
///
/// # Returns
/// The pane ID of the newly created window.
///
/// # Errors
/// Returns an error if the multiplexer isn't running or window creation fails.
pub fn workmux_create_window(params: WorkmuxCreateWindowParams) -> Result<String> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	if !mux.is_running().unwrap_or(false) {
		return Err(Error::from_reason("Multiplexer is not running"));
	}

	let cwd = std::path::PathBuf::from(&params.cwd);
	let create_params = CreateWindowParams {
		prefix:       &params.prefix,
		name:         &params.name,
		cwd:          &cwd,
		after_window: params.after_window.as_deref(),
	};

	mux.create_window(create_params)
		.map_err(|e| Error::from_reason(format!("Failed to create window: {e}")))
}

/// Check if workmux multiplexer is available and running.
///
/// # Returns
/// `true` if a supported multiplexer (tmux, wezterm, kitty) is running.
pub fn workmux_is_available() -> Result<bool> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);
	Ok(mux.is_running().unwrap_or(false))
}

/// Get the current pane ID if running inside a multiplexer.
///
/// # Returns
/// Pane ID string or `None` if not inside a multiplexer pane.
pub fn workmux_current_pane_id() -> Result<Option<String>> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);
	Ok(mux.current_pane_id())
}

/// Set the agent status for the current pane in workmux dashboard.
///
/// This updates the status icon shown in the workmux dashboard and persists
/// the state for cross-session visibility.
///
/// # Parameters
/// - `status`: Agent status (working, waiting, done)
/// - `title`: Optional pane title override (e.g., task summary)
///
/// # Errors
/// Returns an error if not running inside a multiplexer pane.
pub fn workmux_set_agent_status(status: WorkmuxAgentStatus, title: Option<String>) -> Result<()> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	let pane_id = mux
		.current_pane_id()
		.ok_or_else(|| Error::from_reason("Not running inside a multiplexer pane"))?;

	// Convert status for persist and get icon
	let agent_status: AgentStatus = status.into();
	persist_agent_update(mux.as_ref(), &pane_id, Some(agent_status), title);

	// Also update the visual status indicator
	if let Ok(config) = workmux::Config::load(None) {
		let icon = match agent_status {
			AgentStatus::Working => config.status_icons.working(),
			AgentStatus::Waiting => config.status_icons.waiting(),
			AgentStatus::Done => config.status_icons.done(),
		};
		let _ = mux.set_status(&pane_id, icon, false);
	}

	Ok(())
}

/// Clear the agent status for the current pane.
///
/// Removes the status indicator from the pane.
pub fn workmux_clear_agent_status() -> Result<()> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	let pane_id = mux
		.current_pane_id()
		.ok_or_else(|| Error::from_reason("Not running inside a multiplexer pane"))?;

	let _ = mux.clear_status(&pane_id);
	Ok(())
}

/// Agent state information from workmux state store.
pub struct WorkmuxAgentInfo {
	/// Pane identifier.
	pub pane_id:   String,
	/// Working directory path.
	pub workdir:   String,
	/// Current status if set.
	pub status:    Option<WorkmuxAgentStatus>,
	/// Pane title if set.
	pub title:     Option<String>,
	/// Unix timestamp of last status change.
	pub status_ts: Option<f64>,
}

/// List all tracked agents from the workmux state store.
///
/// # Returns
/// Array of agent info objects.
pub fn workmux_list_agents() -> Result<Vec<WorkmuxAgentInfo>> {
	let store = StateStore::new()
		.map_err(|e| Error::from_reason(format!("Failed to open state store: {e}")))?;

	let agents = store
		.list_all_agents()
		.map_err(|e| Error::from_reason(format!("Failed to list agents: {e}")))?;

	Ok(agents
		.into_iter()
		.map(|a| WorkmuxAgentInfo {
			pane_id:   a.pane_key.pane_id,
			workdir:   a.workdir.to_string_lossy().to_string(),
			status:    a.status.map(Into::into),
			title:     a.pane_title,
			status_ts: a.status_ts.map(|ts| ts as f64),
		})
		.collect())
}

/// Send keys (command) to a specific pane.
///
/// # Parameters
/// - `pane_id`: Target pane identifier
/// - `keys`: Keys/command to send
///
/// # Errors
/// Returns an error if the pane doesn't exist or multiplexer isn't running.
pub fn workmux_send_keys(pane_id: &str, keys: &str) -> Result<()> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	if !mux.is_running().unwrap_or(false) {
		return Err(Error::from_reason("Multiplexer is not running"));
	}

	mux.send_keys(pane_id, keys)
		.map_err(|e| Error::from_reason(format!("Failed to send keys: {e}")))?;

	Ok(())
}

/// Capture terminal output from a pane.
///
/// # Parameters
/// - `pane_id`: Target pane identifier
/// - `lines`: Number of lines to capture (default: 50)
///
/// # Returns
/// Captured terminal content or `None` if capture fails.
pub fn workmux_capture_pane(pane_id: &str, lines: Option<u32>) -> Result<Option<String>> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	if !mux.is_running().unwrap_or(false) {
		return Ok(None);
	}

	let line_count = lines.unwrap_or(50) as u16;
	Ok(mux.capture_pane(pane_id, line_count))
}

/// Check if a window with the given name exists.
///
/// # Parameters
/// - `prefix`: Window name prefix (e.g., "wm-")
/// - `name`: Window name (without prefix)
///
/// # Returns
/// `true` if the window exists.
pub fn workmux_window_exists(prefix: &str, name: &str) -> Result<bool> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	if !mux.is_running().unwrap_or(false) {
		return Ok(false);
	}

	Ok(mux.window_exists(prefix, name).unwrap_or(false))
}

/// Select (focus) a window by name.
///
/// # Parameters
/// - `prefix`: Window name prefix
/// - `name`: Window name (without prefix)
///
/// # Errors
/// Returns an error if the window doesn't exist.
pub fn workmux_select_window(prefix: &str, name: &str) -> Result<()> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	mux.select_window(prefix, name)
		.map_err(|e| Error::from_reason(format!("Failed to select window: {e}")))?;

	Ok(())
}

/// Kill a window by its full name.
///
/// # Parameters
/// - `full_name`: Complete window name including prefix
pub fn workmux_kill_window(full_name: &str) -> Result<()> {
	let backend_type = detect_backend();
	let mux = create_backend(backend_type);

	mux.kill_window(full_name)
		.map_err(|e| Error::from_reason(format!("Failed to kill window: {e}")))?;

	Ok(())
}
