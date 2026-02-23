use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// The hidden directory under `$HOME` where all rho state lives (`~/.rho/`).
const APP_DIR: &str = ".rho";

pub struct Config {
	pub api_key:  String,
	pub base_url: String,
	pub is_oauth: bool,
}

impl Config {
	/// Resolve API key from: CLI flag > env var > config file.
	pub fn resolve(cli_api_key: Option<&str>) -> Result<Self> {
		let api_key = if let Some(key) = cli_api_key {
			key.to_owned()
		} else if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
			key
		} else if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
			token
		} else if let Some(key) = Self::read_from_config_file()? {
			key
		} else {
			bail!("No API key found. Set ANTHROPIC_API_KEY environment variable or pass --api-key");
		};

		let is_oauth = api_key.contains("sk-ant-oat");

		Ok(Self {
			api_key,
			base_url: std::env::var("ANTHROPIC_BASE_URL")
				.unwrap_or_else(|_| "https://api.anthropic.com".to_owned()),
			is_oauth,
		})
	}

	fn config_dir() -> Option<PathBuf> {
		dirs::home_dir().map(|h| h.join(APP_DIR).join("config"))
	}

	fn read_from_config_file() -> Result<Option<String>> {
		let Some(config_dir) = Self::config_dir() else {
			return Ok(None);
		};
		let config_path = config_dir.join("config.json");
		if !config_path.exists() {
			return Ok(None);
		}
		let contents = std::fs::read_to_string(&config_path)?;
		let json: serde_json::Value = serde_json::from_str(&contents)?;
		Ok(json
			.get("anthropic")
			.and_then(|a| a.get("apiKey"))
			.and_then(|v| v.as_str())
			.map(str::to_owned))
	}
}

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

	#[test]
	fn test_oauth_detection() {
		let config = Config {
			api_key:  "sk-ant-oat-test".to_owned(),
			base_url: "https://api.anthropic.com".to_owned(),
			is_oauth: true,
		};
		assert!(config.is_oauth);

		let config2 = Config {
			api_key:  "sk-ant-api-test".to_owned(),
			base_url: "https://api.anthropic.com".to_owned(),
			is_oauth: false,
		};
		assert!(!config2.is_oauth);
	}
}
