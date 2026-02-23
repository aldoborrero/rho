//! CWD encoding and path helpers for session storage.
//!
//! Provides utilities to encode working directory paths into safe directory
//! names for session storage, and to resolve default paths for the agent
//! directory, session directories, and blob storage.

use std::path::{Path, PathBuf};

use chrono::Utc;

/// Encode a working directory path into a safe directory name for session
/// storage.
///
/// Ported from the TypeScript `encodeSessionDirName` in `session-manager.ts`.
///
/// The encoding rules are:
///
/// 1. If the path starts with the user's home directory, the home prefix is
///    stripped (without the trailing `/`), all `/` in the remainder are
///    replaced with `-`, and a single `-` is prepended.
///    - `~/Projects/myapp` becomes `--Projects-myapp` (the leading `/` in the
///      relative portion produces the second `-`)
///    - `~` (home dir itself) becomes `-`
///
/// 2. Otherwise (absolute path not under home), `/` is replaced with `-`, and
///    the result is wrapped with `--` prefix and `--` suffix.
///    - `/tmp/foo` becomes `---tmp-foo--`
///
/// 3. On macOS, a `/private` prefix is stripped from canonical paths before
///    comparison with the home directory, since `/Users/x` can appear as
///    `/private/Users/x` after canonicalization.
pub fn encode_session_dir_name(cwd: &Path) -> String {
	let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
	let cwd_str = cwd.to_string_lossy();

	// On macOS, /Users/x can appear as /private/Users/x after canonicalization.
	let clean_cwd = if cwd_str.starts_with("/private") {
		cwd_str.replacen("/private", "", 1)
	} else {
		cwd_str.to_string()
	};

	let home_str = home_dir.to_string_lossy();

	if clean_cwd.starts_with(home_str.as_ref()) {
		// Home-relative: strip home prefix, replace "/" with "-", prepend "-"
		let relative = &clean_cwd[home_str.len()..];
		format!("-{}", relative.replace('/', "-"))
	} else {
		// Absolute: replace "/" with "-", wrap with "--"
		format!("--{}--", clean_cwd.replace('/', "-"))
	}
}

/// Generate a session filename: `<ISO-timestamp>_<snowflake>.jsonl`.
///
/// The timestamp format uses dashes instead of colons for filesystem safety:
/// `2026-02-22T10-30-00-000Z_147224218a4e5a56.jsonl`
pub fn session_file_name(snowflake_id: &str) -> String {
	let now = Utc::now();
	let ts = now.format("%Y-%m-%dT%H-%M-%S-%3fZ");
	format!("{ts}_{snowflake_id}.jsonl")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_encode_home_relative() {
		// A path under the home directory should encode with dash prefix.
		// The leading "/" in the relative portion (after stripping home without
		// trailing slash) becomes a "-", matching TypeScript's slice behavior.
		let home = dirs::home_dir().expect("home dir must exist for test");
		let cwd = home.join("Projects").join("myapp");
		let encoded = encode_session_dir_name(&cwd);
		assert_eq!(encoded, "--Projects-myapp");
	}

	#[test]
	fn test_encode_absolute() {
		// A non-home absolute path should be wrapped with double dashes.
		// The leading "/" also becomes "-", giving "---tmp-foo--".
		let cwd = Path::new("/tmp/foo");
		let encoded = encode_session_dir_name(cwd);

		// Only check the pattern if /tmp/foo is not under the home dir
		let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/nonexistent"));
		if !cwd.starts_with(&home) {
			assert_eq!(encoded, "---tmp-foo--");
		}
	}

	#[test]
	fn test_encode_home_root() {
		// Just the home dir itself should encode to "-".
		// relative = "" (empty), replaceAll is no-op, prepend "-" gives "-".
		let home = dirs::home_dir().expect("home dir must exist for test");
		let encoded = encode_session_dir_name(&home);
		assert_eq!(encoded, "-");
	}

	#[test]
	fn test_session_file_name_format() {
		let name = session_file_name("147224218a4e5a56");
		assert!(name.ends_with(".jsonl"), "expected .jsonl extension, got: {name}");
		assert!(name.contains('_'), "expected underscore separator, got: {name}");
		assert!(name.contains("147224218a4e5a56"), "expected snowflake ID in filename, got: {name}");
		// Verify timestamp format: YYYY-MM-DDTHH-MM-SS-mmmZ
		// The part before the underscore should be the timestamp
		let parts: Vec<&str> = name.splitn(2, '_').collect();
		assert_eq!(parts.len(), 2, "expected exactly one underscore separator");
		let ts_part = parts[0];
		assert!(ts_part.ends_with('Z'), "timestamp should end with Z, got: {ts_part}");
		assert!(ts_part.contains('T'), "timestamp should contain T separator, got: {ts_part}");
	}
}
