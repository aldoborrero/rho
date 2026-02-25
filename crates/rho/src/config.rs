//! Path helpers for rho's hidden directory layout.
//!
//! API key resolution and configuration loading have moved to
//! [`crate::settings`]. This module retains the directory helpers used by
//! session management and blob storage.

use std::path::{Path, PathBuf};

/// The hidden directory under `$HOME` where all rho state lives (`~/.rho/`).
const APP_DIR: &str = ".rho";

/// Get the default agent directory (`~/.rho/agent/`).
pub fn get_default_agent_dir() -> PathBuf {
	dirs::home_dir()
		.unwrap_or_else(|| PathBuf::from("."))
		.join(APP_DIR)
		.join("agent")
}

/// Get the session directory for a given CWD.
///
/// Returns `~/.rho/agent/sessions/<encoded-cwd>/`.
pub fn get_default_session_dir(cwd: &Path) -> PathBuf {
	let encoded = crate::session::paths::encode_session_dir_name(cwd);
	get_default_agent_dir().join("sessions").join(encoded)
}

/// Get the blobs directory (`~/.rho/agent/blobs/`).
pub fn get_blobs_dir() -> PathBuf {
	get_default_agent_dir().join("blobs")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_get_default_agent_dir() {
		let dir = get_default_agent_dir();
		let dir_str = dir.to_string_lossy();
		assert!(
			dir_str.ends_with(".rho/agent"),
			"expected path ending in .rho/agent, got: {dir_str}"
		);
	}

	#[test]
	fn test_get_default_session_dir() {
		let home = dirs::home_dir().expect("home dir must exist for test");
		let cwd = home.join("Projects").join("myapp");
		let dir = get_default_session_dir(&cwd);
		let dir_str = dir.to_string_lossy();

		assert!(dir_str.contains("sessions/"), "expected path containing sessions/, got: {dir_str}");
		assert!(dir_str.contains("--Projects-myapp"), "expected encoded cwd in path, got: {dir_str}");
	}

	#[test]
	fn test_get_blobs_dir() {
		let dir = get_blobs_dir();
		let dir_str = dir.to_string_lossy();
		assert!(
			dir_str.ends_with(".rho/agent/blobs"),
			"expected path ending in .rho/agent/blobs, got: {dir_str}"
		);
	}
}
