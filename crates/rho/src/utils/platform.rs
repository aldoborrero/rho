/// Get the OS name (e.g., `linux`, `macos`, `windows`).
pub const fn os_name() -> &'static str {
	std::env::consts::OS
}

/// Get the CPU architecture (e.g., `x86_64`, `aarch64`).
pub const fn arch() -> &'static str {
	std::env::consts::ARCH
}

/// Get the OS version string.
/// On Linux reads `/etc/os-release` `PRETTY_NAME`, on others returns `None`.
pub fn os_version() -> Option<String> {
	#[cfg(target_os = "linux")]
	{
		if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
			for line in content.lines() {
				if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
					return Some(value.trim_matches('"').to_owned());
				}
			}
		}
		if let Ok(content) = std::fs::read_to_string("/proc/version") {
			let first_line = content.lines().next().unwrap_or("");
			if !first_line.is_empty() {
				return Some(first_line.to_owned());
			}
		}
		None
	}
	#[cfg(not(target_os = "linux"))]
	{
		None
	}
}

/// Get the CPU model name (e.g., "AMD Ryzen 9 5900X").
/// Reads `/proc/cpuinfo` on Linux.
pub fn cpu_model() -> Option<String> {
	#[cfg(target_os = "linux")]
	{
		let content = std::fs::read_to_string("/proc/cpuinfo").ok()?;
		for line in content.lines() {
			if let Some(value) = line.strip_prefix("model name") {
				let value = value.trim_start_matches([' ', '\t', ':']);
				if !value.is_empty() {
					return Some(value.to_owned());
				}
			}
		}
		None
	}
	#[cfg(not(target_os = "linux"))]
	{
		None
	}
}

/// Get the number of logical CPUs.
pub fn cpu_count() -> usize {
	std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Get the terminal name from environment variables.
/// Checks `TERM_PROGRAM` (+ version), `WT_SESSION`, `TERM`, `COLORTERM` in that
/// order.
pub fn terminal() -> Option<String> {
	if let Some(prog) = std::env::var("TERM_PROGRAM").ok().filter(|s| !s.is_empty()) {
		return if let Some(ver) = std::env::var("TERM_PROGRAM_VERSION")
			.ok()
			.filter(|s| !s.is_empty())
		{
			Some(format!("{prog} {ver}"))
		} else {
			Some(prog)
		};
	}
	if std::env::var("WT_SESSION").is_ok() {
		return Some("Windows Terminal".to_owned());
	}
	for var in ["TERM", "COLORTERM", "TERMINAL_EMULATOR"] {
		if let Ok(val) = std::env::var(var) {
			let trimmed = val.trim().to_owned();
			if !trimmed.is_empty() {
				return Some(trimmed);
			}
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn os_name_returns_known_value() {
		let name = os_name();
		assert!(
			[
				"linux",
				"macos",
				"windows",
				"freebsd",
				"openbsd",
				"netbsd",
				"dragonfly",
				"solaris",
				"illumos",
				"android",
				"ios"
			]
			.contains(&name),
			"unexpected OS name: {name}"
		);
	}

	#[test]
	fn arch_is_not_empty() {
		assert!(!arch().is_empty());
	}

	#[test]
	fn cpu_count_is_at_least_one() {
		assert!(cpu_count() >= 1);
	}

	#[test]
	fn terminal_reads_env() {
		let original = std::env::var("TERM_PROGRAM").ok();
		// SAFETY: test is single-threaded for env manipulation
		unsafe {
			std::env::set_var("TERM_PROGRAM", "TestTerminal");
		}
		let result = terminal();
		match original {
			Some(val) => unsafe { std::env::set_var("TERM_PROGRAM", val) },
			None => unsafe { std::env::remove_var("TERM_PROGRAM") },
		}
		assert!(result.is_some());
		assert!(result.unwrap().contains("TestTerminal"));
	}

	#[test]
	fn terminal_with_version() {
		let orig_prog = std::env::var("TERM_PROGRAM").ok();
		let orig_ver = std::env::var("TERM_PROGRAM_VERSION").ok();
		// SAFETY: test is single-threaded for env manipulation
		unsafe {
			std::env::set_var("TERM_PROGRAM", "vscode");
			std::env::set_var("TERM_PROGRAM_VERSION", "1.85");
		}
		let result = terminal();
		match orig_prog {
			Some(val) => unsafe { std::env::set_var("TERM_PROGRAM", val) },
			None => unsafe { std::env::remove_var("TERM_PROGRAM") },
		}
		match orig_ver {
			Some(val) => unsafe { std::env::set_var("TERM_PROGRAM_VERSION", val) },
			None => unsafe { std::env::remove_var("TERM_PROGRAM_VERSION") },
		}
		assert_eq!(result, Some("vscode 1.85".to_owned()));
	}
}
