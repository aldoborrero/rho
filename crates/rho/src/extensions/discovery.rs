use std::path::{Path, PathBuf};

use super::{ExtensionManifest, manager::ExtensionManager};
use crate::settings::Settings;

/// Discover and load extensions from standard paths.
///
/// Discovery order:
/// 1. `~/.rho/extensions/*/extension.toml` (global)
/// 2. `.rho/extensions/*/extension.toml` (project-local)
/// 3. Paths from `settings.extensions.extra_paths`
///
/// Extensions with a `[runtime]` section specifying `engine = "lua"` are loaded
/// via the Luau scripting runtime. Extensions without a runtime section are
/// logged as discovered (manifest-only).
pub fn discover_and_load(
	manager: &mut ExtensionManager,
	settings: &Settings,
) -> anyhow::Result<()> {
	if !settings.extensions.enabled {
		return Ok(());
	}

	let manifests = discover_manifests(settings);

	for (path, manifest) in manifests {
		if settings.extensions.disabled.contains(&manifest.id) {
			continue;
		}

		match &manifest.runtime {
			Some(rc) if rc.engine == "lua" => {
				let ext_dir = path.parent().unwrap_or(Path::new("."));
				match super::lua::load_lua_extension(
					manifest.clone(),
					ext_dir.to_owned(),
					rc.entry.clone(),
				) {
					Ok(ext) => {
						eprintln!("[extensions] loaded Lua: {} v{}", manifest.name, manifest.version);
						manager.load(ext);
					},
					Err(e) => {
						eprintln!("[extensions] failed to load {}: {e}", manifest.id);
					},
				}
			},
			Some(rc) => {
				eprintln!("[extensions] unsupported engine '{}' for {}", rc.engine, manifest.id);
			},
			None => {
				eprintln!(
					"[extensions] discovered (no runtime): {} v{}",
					manifest.name, manifest.version
				);
			},
		}
	}

	Ok(())
}

/// Scan standard directories for `extension.toml` manifests.
///
/// Returns `(manifest_path, parsed_manifest)` pairs.
fn discover_manifests(settings: &Settings) -> Vec<(PathBuf, ExtensionManifest)> {
	let mut results = Vec::new();

	// 1. Global: ~/.rho/extensions/*/extension.toml
	if let Some(home) = dirs::home_dir() {
		scan_extensions_dir(&home.join(".rho").join("extensions"), &mut results);
	}

	// 2. Project-local: .rho/extensions/*/extension.toml
	if let Ok(cwd) = std::env::current_dir() {
		scan_extensions_dir(&cwd.join(".rho").join("extensions"), &mut results);
	}

	// 3. Extra paths from settings.
	for extra in &settings.extensions.extra_paths {
		scan_extensions_dir(Path::new(extra), &mut results);
	}

	results
}

/// Scan a directory for subdirectories containing `extension.toml`.
fn scan_extensions_dir(dir: &Path, results: &mut Vec<(PathBuf, ExtensionManifest)>) {
	let entries = match std::fs::read_dir(dir) {
		Ok(entries) => entries,
		Err(_) => return, // Directory doesn't exist or isn't readable.
	};

	for entry in entries.flatten() {
		let manifest_path = entry.path().join("extension.toml");
		if manifest_path.is_file() {
			match load_manifest(&manifest_path) {
				Ok(manifest) => results.push((manifest_path, manifest)),
				Err(e) => {
					eprintln!("[extensions] failed to parse {}: {e}", manifest_path.display());
				},
			}
		}
	}
}

/// Parse an `extension.toml` file into an [`ExtensionManifest`].
fn load_manifest(path: &Path) -> anyhow::Result<ExtensionManifest> {
	let contents = std::fs::read_to_string(path)?;
	let manifest: ExtensionManifest = toml::from_str(&contents)?;
	Ok(manifest)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::settings::ExtensionSettings;

	#[test]
	fn discover_and_load_disabled_is_noop() {
		let mut mgr = ExtensionManager::new();
		let mut settings = Settings::default();
		settings.extensions.enabled = false;
		assert!(discover_and_load(&mut mgr, &settings).is_ok());
	}

	#[test]
	fn discover_and_load_enabled_with_no_dirs_is_ok() {
		let mut mgr = ExtensionManager::new();
		let settings = Settings::default();
		assert!(discover_and_load(&mut mgr, &settings).is_ok());
	}

	#[test]
	fn scan_nonexistent_dir_returns_empty() {
		let mut results = Vec::new();
		scan_extensions_dir(Path::new("/nonexistent_dir_12345"), &mut results);
		assert!(results.is_empty());
	}

	#[test]
	fn load_manifest_parses_valid_toml() {
		let dir = std::env::temp_dir().join("rho_test_extension_manifest");
		let _ = std::fs::create_dir_all(&dir);
		let manifest_path = dir.join("extension.toml");
		std::fs::write(
			&manifest_path,
			r#"
id = "test-ext"
name = "Test Extension"
version = "0.1.0"
description = "A test"
"#,
		)
		.unwrap();

		let manifest = load_manifest(&manifest_path).unwrap();
		assert_eq!(manifest.id, "test-ext");
		assert_eq!(manifest.name, "Test Extension");
		assert_eq!(manifest.version, "0.1.0");
		assert_eq!(manifest.description, "A test");

		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn load_manifest_rejects_invalid_toml() {
		let dir = std::env::temp_dir().join("rho_test_extension_bad");
		let _ = std::fs::create_dir_all(&dir);
		let manifest_path = dir.join("extension.toml");
		std::fs::write(&manifest_path, "not valid toml {{{{").unwrap();

		assert!(load_manifest(&manifest_path).is_err());

		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn scan_dir_finds_manifests() {
		let base = std::env::temp_dir().join("rho_test_scan_extensions");
		let ext_dir = base.join("my-ext");
		let _ = std::fs::create_dir_all(&ext_dir);
		std::fs::write(
			ext_dir.join("extension.toml"),
			r#"
id = "my-ext"
name = "My Extension"
version = "1.0.0"
"#,
		)
		.unwrap();

		let mut results = Vec::new();
		scan_extensions_dir(&base, &mut results);
		assert_eq!(results.len(), 1);
		assert_eq!(results[0].1.id, "my-ext");

		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn disabled_extensions_are_skipped() {
		let base = std::env::temp_dir().join("rho_test_disabled_ext");
		let ext_dir = base.join("skip-me");
		let _ = std::fs::create_dir_all(&ext_dir);
		std::fs::write(
			ext_dir.join("extension.toml"),
			r#"
id = "skip-me"
name = "Skipped"
version = "0.1.0"
"#,
		)
		.unwrap();

		let mut mgr = ExtensionManager::new();
		let mut settings = Settings::default();
		settings.extensions.extra_paths = vec![base.to_string_lossy().into_owned()];
		settings.extensions.disabled = vec!["skip-me".to_owned()];

		// Should not error even though the extension exists — it's disabled.
		assert!(discover_and_load(&mut mgr, &settings).is_ok());

		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn extension_settings_defaults() {
		let s = ExtensionSettings::default();
		assert!(s.enabled);
		assert!(s.extra_paths.is_empty());
		assert!(s.disabled.is_empty());
	}
}
