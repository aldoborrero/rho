//! Layered TOML-based settings with global/project merging.
//!
//! Load order:
//! 1. `~/.rho/config.toml` (global defaults)
//! 2. `.rho/config.toml` (project overrides)
//! 3. CLI flags (highest priority)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::Cli;

// ---------------------------------------------------------------------------
// Settings structs
// ---------------------------------------------------------------------------

/// Top-level settings, deserialized from TOML with layered merging.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
	pub model:      ModelSettings,
	pub agent:      AgentSettings,
	pub compaction: CompactionSettings,
	pub retry:      RetrySettings,

	/// Resolved at load time, not from TOML.
	#[serde(skip)]
	pub api_key:  String,
	/// Resolved at load time, not from TOML.
	#[serde(skip)]
	pub base_url: String,
	/// Whether the API key is an OAuth token.
	#[serde(skip)]
	pub is_oauth: bool,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			model:      ModelSettings::default(),
			agent:      AgentSettings::default(),
			compaction: CompactionSettings::default(),
			retry:      RetrySettings::default(),
			api_key:    String::new(),
			base_url:   "https://api.anthropic.com".to_owned(),
			is_oauth:   false,
		}
	}
}

/// Model role assignments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSettings {
	/// Default model (used when no role is specified).
	pub default: String,
	/// Fast/cheap model.
	pub smol:    String,
	/// Thinking/powerful model.
	pub slow:    String,
}

impl Default for ModelSettings {
	fn default() -> Self {
		Self {
			default: "claude-sonnet-4-5-20250929".to_owned(),
			smol:    "claude-haiku-4-5-20251001".to_owned(),
			slow:    "claude-opus-4-6".to_owned(),
		}
	}
}

/// Agent loop parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
	pub max_tokens:  u32,
	/// "off", "low", "medium", "high"
	pub thinking:    String,
	/// -1.0 = provider default, 0.0..2.0 = custom
	pub temperature: f32,
}

impl Default for AgentSettings {
	fn default() -> Self {
		Self { max_tokens: 8192, thinking: "off".to_owned(), temperature: -1.0 }
	}
}

/// Compaction settings (replaces the local
/// `compaction::settings::CompactionSettings`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactionSettings {
	pub enabled:            bool,
	pub reserve_tokens:     u32,
	pub keep_recent_tokens: u32,
}

impl Default for CompactionSettings {
	fn default() -> Self {
		Self { enabled: true, reserve_tokens: 16_384, keep_recent_tokens: 20_000 }
	}
}

/// Retry settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrySettings {
	pub max_retries:   u32,
	pub base_delay_ms: u64,
}

impl Default for RetrySettings {
	fn default() -> Self {
		Self { max_retries: 3, base_delay_ms: 2000 }
	}
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Path to the global config file (`~/.rho/config.toml`).
pub fn global_config_path() -> Option<PathBuf> {
	dirs::home_dir().map(|h| h.join(".rho").join("config.toml"))
}

/// Path to the project config file (`.rho/config.toml` relative to cwd).
fn project_config_path() -> Option<PathBuf> {
	let cwd = std::env::current_dir().ok()?;
	let path = cwd.join(".rho").join("config.toml");
	if path.exists() { Some(path) } else { None }
}

/// Load settings with layered merging: global → project → CLI overrides.
pub fn load(cli: &Cli) -> Result<Settings> {
	// Start with global TOML if it exists.
	let mut merged = load_toml_value(global_config_path().as_deref());

	// Merge project TOML on top.
	if let Some(project_path) = project_config_path() {
		let project = load_toml_value(Some(&project_path));
		merge_toml(&mut merged, &project);
	}

	// Deserialize the merged TOML into Settings.
	let mut settings: Settings = match merged.try_into() {
		Ok(s) => s,
		Err(e) => {
			eprintln!("Warning: config parse error, using defaults: {e}");
			Settings::default()
		},
	};

	// Resolve API key: CLI → env → legacy config.json
	settings.api_key = resolve_api_key(cli.api_key.as_deref())?;
	settings.is_oauth = settings.api_key.contains("sk-ant-oat");
	settings.base_url = std::env::var("ANTHROPIC_BASE_URL")
		.unwrap_or_else(|_| "https://api.anthropic.com".to_owned());

	// CLI overrides.
	if let Some(ref model) = cli.model {
		settings.model.default.clone_from(model);
	}
	if let Some(ref thinking) = cli.thinking {
		settings.agent.thinking.clone_from(thinking);
	}

	Ok(settings)
}

/// Reload settings from disk (re-merge global + project), then re-apply CLI
/// overrides. Used after `/config set` mutates the global file.
pub fn reload(cli: &Cli) -> Result<Settings> {
	load(cli)
}

// ---------------------------------------------------------------------------
// get / set / reset — for /config commands
// ---------------------------------------------------------------------------

/// Read a dotted path (e.g. `"agent.max_tokens"`) from the merged config.
pub fn get(path: &str) -> Option<String> {
	let mut merged = load_toml_value(global_config_path().as_deref());
	if let Some(project_path) = project_config_path() {
		let project = load_toml_value(Some(&project_path));
		merge_toml(&mut merged, &project);
	}
	resolve_dotted(&merged, path)
}

/// Write a value to the global config file at the given dotted path.
pub fn set(path: &str, value: &str) -> Result<()> {
	let Some(config_path) = global_config_path() else {
		bail!("Cannot determine home directory");
	};

	let mut doc = load_toml_value(Some(&config_path));
	set_dotted(&mut doc, path, value)?;

	write_toml(&config_path, &doc)
}

/// Remove a key from the global config file.
pub fn reset(path: &str) -> Result<()> {
	let Some(config_path) = global_config_path() else {
		bail!("Cannot determine home directory");
	};

	let mut doc = load_toml_value(Some(&config_path));
	remove_dotted(&mut doc, path);

	write_toml(&config_path, &doc)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// API key resolution: CLI → `ANTHROPIC_API_KEY` → `ANTHROPIC_OAUTH_TOKEN`.
fn resolve_api_key(cli_key: Option<&str>) -> Result<String> {
	if let Some(key) = cli_key {
		return Ok(key.to_owned());
	}
	if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
		return Ok(key);
	}
	if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
		return Ok(token);
	}
	bail!("No API key found. Set ANTHROPIC_API_KEY environment variable or pass --api-key");
}

/// Load a TOML file into a `toml::Value`. Returns an empty table if the
/// file doesn't exist.
fn load_toml_value(path: Option<&Path>) -> toml::Value {
	let Some(path) = path else {
		return toml::Value::Table(toml::map::Map::new());
	};
	match std::fs::read_to_string(path) {
		Ok(contents) => contents
			.parse::<toml::Value>()
			.unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new())),
		Err(_) => toml::Value::Table(toml::map::Map::new()),
	}
}

/// Deep-merge `overlay` into `base`. Tables are merged recursively; scalars
/// in `overlay` replace those in `base`.
fn merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
	match (base, overlay) {
		(toml::Value::Table(base_tbl), toml::Value::Table(overlay_tbl)) => {
			for (key, overlay_val) in overlay_tbl {
				if let Some(base_val) = base_tbl.get_mut(key) {
					merge_toml(base_val, overlay_val);
				} else {
					base_tbl.insert(key.clone(), overlay_val.clone());
				}
			}
		},
		(base, overlay) => {
			// Scalar or type-mismatch: overlay wins.
			*base = overlay.clone();
		},
	}
}

/// Resolve a dotted path like `"agent.max_tokens"` into its string value.
fn resolve_dotted(value: &toml::Value, path: &str) -> Option<String> {
	let mut current = value;
	for key in path.split('.') {
		current = current.as_table()?.get(key)?;
	}
	Some(toml_value_to_string(current))
}

/// Set a value at a dotted path, creating intermediate tables as needed.
fn set_dotted(root: &mut toml::Value, path: &str, raw: &str) -> Result<()> {
	let parts: Vec<&str> = path.split('.').collect();
	let mut current = root;
	for &key in &parts[..parts.len() - 1] {
		if !current.as_table().is_some_and(|t| t.contains_key(key)) {
			let tbl = current
				.as_table_mut()
				.context("intermediate path element is not a table")?;
			tbl.insert(key.to_owned(), toml::Value::Table(toml::map::Map::new()));
		}
		current = current
			.as_table_mut()
			.and_then(|t| t.get_mut(key))
			.context("failed to traverse config path")?;
	}
	let leaf_key = parts.last().context("config path must not be empty")?;
	let tbl = current
		.as_table_mut()
		.context("leaf parent is not a table")?;
	tbl.insert((*leaf_key).to_owned(), parse_toml_value(raw));
	Ok(())
}

/// Remove a key at a dotted path.
fn remove_dotted(root: &mut toml::Value, path: &str) {
	if path.is_empty() {
		return;
	}
	let parts: Vec<&str> = path.split('.').collect();
	let mut current = root;
	for &key in &parts[..parts.len() - 1] {
		match current.as_table_mut().and_then(|t| t.get_mut(key)) {
			Some(next) => current = next,
			None => return,
		}
	}
	if let Some(tbl) = current.as_table_mut() {
		if let Some(leaf_key) = parts.last() {
			tbl.remove(*leaf_key);
		}
	}
}

/// Write a `toml::Value` to a file, creating parent dirs as needed.
fn write_toml(path: &Path, value: &toml::Value) -> Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let contents = toml::to_string_pretty(value)?;
	std::fs::write(path, contents)?;
	Ok(())
}

/// Parse a raw string into a typed `toml::Value` (bool → int → float → string).
fn parse_toml_value(raw: &str) -> toml::Value {
	if let Ok(b) = raw.parse::<bool>() {
		return toml::Value::Boolean(b);
	}
	if let Ok(i) = raw.parse::<i64>() {
		return toml::Value::Integer(i);
	}
	if let Ok(f) = raw.parse::<f64>() {
		return toml::Value::Float(f);
	}
	toml::Value::String(raw.to_owned())
}

/// Format a `toml::Value` for display.
fn toml_value_to_string(v: &toml::Value) -> String {
	match v {
		toml::Value::String(s) => s.clone(),
		toml::Value::Integer(i) => i.to_string(),
		toml::Value::Float(f) => f.to_string(),
		toml::Value::Boolean(b) => b.to_string(),
		other => other.to_string(),
	}
}

/// Return all settings as a flat list of `(dotted_key, value)` pairs for
/// display.
pub fn list_all() -> Vec<(String, String)> {
	let mut merged = load_toml_value(global_config_path().as_deref());
	if let Some(project_path) = project_config_path() {
		let project = load_toml_value(Some(&project_path));
		merge_toml(&mut merged, &project);
	}

	// If we have no config files at all, show defaults by round-tripping
	// through Settings::default().
	let effective = if merged.as_table().is_some_and(toml::map::Map::is_empty) {
		let defaults = Settings::default();
		toml::Value::try_from(&defaults).unwrap_or(merged)
	} else {
		merged
	};

	let mut out = Vec::new();
	flatten_toml("", &effective, &mut out);
	out.sort_by(|a, b| a.0.cmp(&b.0));
	out
}

/// Recursively flatten a TOML value into dotted-key/string-value pairs.
fn flatten_toml(prefix: &str, value: &toml::Value, out: &mut Vec<(String, String)>) {
	match value {
		toml::Value::Table(tbl) => {
			for (key, val) in tbl {
				let full_key = if prefix.is_empty() {
					key.clone()
				} else {
					format!("{prefix}.{key}")
				};
				flatten_toml(&full_key, val, out);
			}
		},
		_ => {
			out.push((prefix.to_owned(), toml_value_to_string(value)));
		},
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_settings_round_trip() {
		let s = Settings::default();
		assert_eq!(s.agent.max_tokens, 8192);
		assert_eq!(s.agent.thinking, "off");
		assert!(s.compaction.enabled);
		assert_eq!(s.retry.max_retries, 3);
		assert_eq!(s.model.default, "claude-sonnet-4-5-20250929");
	}

	#[test]
	fn merge_toml_tables() {
		let mut base: toml::Value = toml::from_str(
			r#"
			[agent]
			max_tokens = 8192
			thinking = "off"
			"#,
		)
		.unwrap();
		let overlay: toml::Value = toml::from_str(
			r#"
			[agent]
			max_tokens = 16384
			"#,
		)
		.unwrap();
		merge_toml(&mut base, &overlay);
		let tbl = base.as_table().unwrap();
		let agent = tbl.get("agent").unwrap().as_table().unwrap();
		assert_eq!(agent.get("max_tokens").unwrap().as_integer(), Some(16384));
		assert_eq!(
			agent.get("thinking").unwrap().as_str(),
			Some("off"),
			"non-overlapping key should be preserved"
		);
	}

	#[test]
	fn resolve_dotted_path() {
		let value: toml::Value = toml::from_str(
			r#"
			[agent]
			max_tokens = 8192
			"#,
		)
		.unwrap();
		assert_eq!(resolve_dotted(&value, "agent.max_tokens"), Some("8192".to_owned()));
		assert_eq!(resolve_dotted(&value, "agent.nonexistent"), None);
	}

	#[test]
	fn set_and_remove_dotted() {
		let mut value = toml::Value::Table(toml::map::Map::new());
		set_dotted(&mut value, "agent.max_tokens", "16384").unwrap();
		assert_eq!(resolve_dotted(&value, "agent.max_tokens"), Some("16384".to_owned()));

		remove_dotted(&mut value, "agent.max_tokens");
		assert_eq!(resolve_dotted(&value, "agent.max_tokens"), None);
	}

	#[test]
	fn remove_dotted_empty_path_does_not_panic() {
		let mut value = toml::Value::Table(toml::map::Map::new());
		// Insert a key named "" to verify it is NOT removed by an empty path call.
		value
			.as_table_mut()
			.unwrap()
			.insert(String::new(), toml::Value::Boolean(true));
		// Must not panic — should be a no-op.
		remove_dotted(&mut value, "");
		// The empty-string key should still be present (not accidentally removed).
		assert!(value.as_table().unwrap().contains_key(""));
	}

	#[test]
	fn parse_toml_value_types() {
		assert_eq!(parse_toml_value("true"), toml::Value::Boolean(true));
		assert_eq!(parse_toml_value("42"), toml::Value::Integer(42));
		assert_eq!(parse_toml_value("3.14"), toml::Value::Float(3.14));
		assert_eq!(parse_toml_value("hello"), toml::Value::String("hello".to_owned()));
	}

	#[test]
	fn flatten_produces_dotted_keys() {
		let value: toml::Value = toml::from_str(
			r#"
			[model]
			default = "sonnet"
			[agent]
			max_tokens = 8192
			"#,
		)
		.unwrap();
		let mut out = Vec::new();
		flatten_toml("", &value, &mut out);
		assert!(
			out.iter()
				.any(|(k, v)| k == "model.default" && v == "sonnet")
		);
		assert!(
			out.iter()
				.any(|(k, v)| k == "agent.max_tokens" && v == "8192")
		);
	}

	#[test]
	fn settings_deserializes_from_partial_toml() {
		let toml_str = r#"
		[agent]
		max_tokens = 16384
		"#;
		let settings: Settings = toml::from_str(toml_str).unwrap();
		assert_eq!(settings.agent.max_tokens, 16384);
		// Other fields should be defaults
		assert_eq!(settings.agent.thinking, "off");
		assert_eq!(settings.model.default, "claude-sonnet-4-5-20250929");
	}

	#[test]
	fn list_all_shows_defaults() {
		let items = list_all();
		// Should at least have model.default, agent.max_tokens, etc.
		assert!(!items.is_empty());
	}
}
