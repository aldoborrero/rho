//! `/profile` command handler.

use std::path::PathBuf;

use rho_tools::prof::get_work_profile;

use super::super::types::{CommandContext, CommandResult};

/// Parse args: `/profile [seconds] [path]`
///
/// Both arguments are optional and order-insensitive: a numeric token is
/// treated as seconds, anything else as a file path.
fn parse_args(args: &str) -> (f64, Option<PathBuf>) {
	let mut seconds = 10.0;
	let mut path = None;

	for token in args.split_whitespace() {
		if let Ok(s) = token.parse::<f64>() {
			seconds = s;
		} else {
			path = Some(PathBuf::from(token));
		}
	}

	(seconds, path)
}

/// Default profile directory under `.rho/profile/`.
fn profile_dir() -> PathBuf {
	let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
	cwd.join(".rho").join("profile")
}

/// Open a file in the default browser/viewer.
fn open_in_browser(path: &std::path::Path) {
	#[cfg(target_os = "linux")]
	let cmd = "xdg-open";
	#[cfg(target_os = "macos")]
	let cmd = "open";
	#[cfg(not(any(target_os = "linux", target_os = "macos")))]
	let cmd = "xdg-open";

	let _ = std::process::Command::new(cmd)
		.arg(path)
		.stdin(std::process::Stdio::null())
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn();
}

/// Handler for `/profile` — show streaming performance profile.
pub fn cmd_profile(ctx: &CommandContext<'_>) -> CommandResult {
	let (seconds, user_path) = parse_args(ctx.args);
	let profile = get_work_profile(seconds);

	if profile.sample_count == 0 {
		return CommandResult::Message(
			"No profiling samples collected. Try streaming some text first.".to_owned(),
		);
	}

	// Save SVG flamegraph if available.
	let svg_note = if let Some(ref svg) = profile.svg {
		let path = match user_path {
			Some(p) => p,
			None => {
				let dir = profile_dir();
				let _ = std::fs::create_dir_all(&dir);
				let id: u64 = rand::random();
				dir.join(format!("rho-profile-{id:016x}.svg"))
			},
		};
		if std::fs::write(&path, svg).is_ok() {
			open_in_browser(&path);
			format!("\n\nFlamegraph saved to: {}", path.display())
		} else {
			format!("\n\nFailed to write flamegraph to: {}", path.display())
		}
	} else {
		String::new()
	};

	CommandResult::Message(format!("{}{svg_note}", profile.summary))
}
