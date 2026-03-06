//! Terminal breadcrumb support.
//!
//! Breadcrumbs link the current terminal to a session file so that
//! `continue_recent()` can quickly find the most recent session without
//! scanning the entire sessions directory.
//!
//! File location: `~/.rho/agent/terminal-sessions/<terminal-id>`
//! Content format: `<cwd>\n<session-file-path>\n`

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Write a breadcrumb linking the current terminal to a session file.
///
/// File location: `~/.rho/agent/terminal-sessions/<terminal-id>`
/// Content: `<cwd>\n<session-file-path>\n`
///
/// This is best-effort: failures are logged to stderr but do not propagate.
pub fn write_breadcrumb(cwd: &Path, session_file: &Path) {
	let Some(terminal_id) = get_terminal_id() else {
		return;
	};
	let breadcrumb_dir = crate::config::get_default_agent_dir().join("terminal-sessions");
	write_breadcrumb_to(&breadcrumb_dir, &terminal_id, cwd, session_file);
}

/// Read the breadcrumb for the current terminal, scoped to a CWD.
///
/// Returns the session file path if:
/// 1. A breadcrumb exists for the current terminal
/// 2. The CWD in the breadcrumb matches the given CWD
pub fn read_breadcrumb(cwd: &Path) -> Option<PathBuf> {
	let terminal_id = get_terminal_id()?;
	let breadcrumb_dir = crate::config::get_default_agent_dir().join("terminal-sessions");
	read_breadcrumb_from(&breadcrumb_dir, &terminal_id, cwd)
}

// ---------------------------------------------------------------------------
// Testable variants (accept explicit parameters)
// ---------------------------------------------------------------------------

/// Write a breadcrumb to a specific directory (for testing).
pub fn write_breadcrumb_to(
	breadcrumb_dir: &Path,
	terminal_id: &str,
	cwd: &Path,
	session_file: &Path,
) {
	let breadcrumb_file = breadcrumb_dir.join(terminal_id);
	let content = format!("{}\n{}\n", cwd.display(), session_file.display());
	if let Err(e) = std::fs::create_dir_all(breadcrumb_dir) {
		eprintln!("Warning: failed to create breadcrumb directory {}: {e}", breadcrumb_dir.display());
		return;
	}
	if let Err(e) = std::fs::write(&breadcrumb_file, content) {
		eprintln!("Warning: failed to write breadcrumb file {}: {e}", breadcrumb_file.display());
	}
}

/// Read a breadcrumb from a specific directory (for testing).
pub fn read_breadcrumb_from(
	breadcrumb_dir: &Path,
	terminal_id: &str,
	cwd: &Path,
) -> Option<PathBuf> {
	let breadcrumb_file = breadcrumb_dir.join(terminal_id);
	let content = std::fs::read_to_string(&breadcrumb_file).ok()?;
	let mut lines = content.lines();
	let breadcrumb_cwd = lines.next()?;
	let session_file = lines.next()?;

	// Only return if CWD matches
	if Path::new(breadcrumb_cwd) != cwd {
		return None;
	}

	Some(PathBuf::from(session_file))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the terminal ID from the environment.
///
/// Checks these environment variables in order:
/// 1. `TERM_SESSION_ID` (macOS Terminal.app)
/// 2. `WINDOWID` (X11 terminals)
/// 3. `WT_SESSION` (Windows Terminal)
///
/// Falls back to the TTY device name if available.
/// The result is sanitized for use as a filename (`/` replaced with `-`).
fn get_terminal_id() -> Option<String> {
	for var in &["TERM_SESSION_ID", "WINDOWID", "WT_SESSION"] {
		if let Ok(val) = std::env::var(var)
			&& !val.is_empty()
		{
			return Some(sanitize_for_filename(&val));
		}
	}

	// Fall back to TTY device name on Unix
	#[cfg(unix)]
	{
		if let Ok(tty) = std::fs::read_link("/proc/self/fd/0") {
			return Some(sanitize_for_filename(&tty.to_string_lossy()));
		}
	}

	None
}

fn sanitize_for_filename(s: &str) -> String {
	s.replace(['/', '\\'], "-")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use tempfile::TempDir;

	use super::*;

	#[test]
	fn test_write_and_read_breadcrumb() {
		let tmp = TempDir::new().unwrap();
		let breadcrumb_dir = tmp.path().join("terminal-sessions");
		let cwd = tmp.path().join("my-project");

		// Create a fake session file so that `is_file()` succeeds.
		let session_file = tmp.path().join("session.jsonl");
		std::fs::write(&session_file, "[]").unwrap();

		let terminal_id = "test-terminal-42";

		write_breadcrumb_to(&breadcrumb_dir, terminal_id, &cwd, &session_file);
		let result = read_breadcrumb_from(&breadcrumb_dir, terminal_id, &cwd);

		assert_eq!(result, Some(session_file));
	}

	#[test]
	fn test_read_breadcrumb_wrong_cwd() {
		let tmp = TempDir::new().unwrap();
		let breadcrumb_dir = tmp.path().join("terminal-sessions");
		let cwd_a = tmp.path().join("project-a");
		let cwd_b = tmp.path().join("project-b");

		let session_file = tmp.path().join("session.jsonl");
		std::fs::write(&session_file, "[]").unwrap();

		let terminal_id = "test-terminal-99";

		write_breadcrumb_to(&breadcrumb_dir, terminal_id, &cwd_a, &session_file);
		let result = read_breadcrumb_from(&breadcrumb_dir, terminal_id, &cwd_b);

		assert_eq!(result, None);
	}

	#[test]
	fn test_read_breadcrumb_missing_file() {
		let tmp = TempDir::new().unwrap();
		let breadcrumb_dir = tmp.path().join("terminal-sessions");
		let cwd = tmp.path().join("my-project");

		// Create the session file, write the breadcrumb, then delete it.
		let session_file = tmp.path().join("session.jsonl");
		std::fs::write(&session_file, "[]").unwrap();

		let terminal_id = "test-terminal-deleted";

		write_breadcrumb_to(&breadcrumb_dir, terminal_id, &cwd, &session_file);

		// Remove the session file before reading.
		std::fs::remove_file(&session_file).unwrap();

		let result = read_breadcrumb_from(&breadcrumb_dir, terminal_id, &cwd);
		assert_eq!(result, Some(session_file));
	}

	#[test]
	fn test_read_breadcrumb_returns_path_even_if_file_missing() {
		let tmp = TempDir::new().unwrap();
		let breadcrumb_dir = tmp.path().join("terminal-sessions");
		let cwd = tmp.path().join("my-project");

		let session_file = tmp.path().join("nonexistent-session.jsonl");
		let breadcrumb_file = breadcrumb_dir.join("test-terminal");
		std::fs::create_dir_all(&breadcrumb_dir).unwrap();
		std::fs::write(&breadcrumb_file, format!("{}\n{}\n", cwd.display(), session_file.display()))
			.unwrap();

		let result = read_breadcrumb_from(&breadcrumb_dir, "test-terminal", &cwd);
		assert_eq!(result, Some(session_file));
	}

	#[test]
	fn test_write_breadcrumb_is_best_effort() {
		// Writing to a path that cannot be created should not panic.
		// On Unix, /proc/1 is not writable by a regular user.
		let impossible_dir = Path::new("/proc/1/breadcrumbs");
		let cwd = Path::new("/tmp/test");
		let session_file = Path::new("/tmp/session.jsonl");

		// This must not panic.
		write_breadcrumb_to(impossible_dir, "term", cwd, session_file);
	}

	#[test]
	fn test_sanitize_for_filename() {
		assert_eq!(sanitize_for_filename("/dev/pts/0"), "-dev-pts-0");
		assert_eq!(sanitize_for_filename("simple"), "simple");
		assert_eq!(sanitize_for_filename("C:\\Users\\me"), "C:-Users-me");
		assert_eq!(sanitize_for_filename("/"), "-");
	}
}
