use std::path::Path;

use super::types::ContextFile;

/// Try to read a file, returning `None` if missing or whitespace-only.
fn read_nonempty(path: &Path) -> Option<String> {
	let content = std::fs::read_to_string(path).ok()?;
	if content.trim().is_empty() {
		return None;
	}
	Some(content)
}

/// Gather CLAUDE.md context files from standard locations.
///
/// Searches (in order):
/// 1. `~/.claude/CLAUDE.md` (user-level)
/// 2. `<cwd>/CLAUDE.md` (project root)
/// 3. `<cwd>/.claude/CLAUDE.md` (project .claude dir)
pub fn gather(cwd: &Path) -> Vec<ContextFile> {
	let mut files = Vec::new();

	// 1. User-level: ~/.claude/CLAUDE.md
	if let Some(home) = dirs::home_dir() {
		let path = home.join(".claude").join("CLAUDE.md");
		if let Some(content) = read_nonempty(&path) {
			files.push(ContextFile { path: path.display().to_string(), content });
		}
	}

	// 2. Project root: <cwd>/CLAUDE.md
	let project_claude = cwd.join("CLAUDE.md");
	if let Some(content) = read_nonempty(&project_claude) {
		files.push(ContextFile { path: project_claude.display().to_string(), content });
	}

	// 3. Project .claude dir: <cwd>/.claude/CLAUDE.md
	let dotclaude = cwd.join(".claude").join("CLAUDE.md");
	if let Some(content) = read_nonempty(&dotclaude) {
		files.push(ContextFile { path: dotclaude.display().to_string(), content });
	}

	files
}

/// Load system prompt customization from `~/.claude/SYSTEM.md`.
pub fn load_system_prompt_customization() -> Option<String> {
	let home = dirs::home_dir()?;
	read_nonempty(&home.join(".claude").join("SYSTEM.md"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gather_does_not_crash_on_nonexistent_dir() {
		let result = gather(Path::new("/nonexistent/dir/that/should/not/exist"));
		// Should return empty (or with user-level files), not panic
		let _ = result;
	}

	#[test]
	fn gather_finds_claude_md_in_tempdir() {
		let dir = tempfile::tempdir().unwrap();
		let claude_md = dir.path().join("CLAUDE.md");
		std::fs::write(&claude_md, "# Test context\nSome instructions.").unwrap();

		let result = gather(dir.path());
		// May also contain ~/.claude/CLAUDE.md, so check project file is included
		let project_files: Vec<_> = result
			.iter()
			.filter(|f| f.path.contains(dir.path().to_str().unwrap()))
			.collect();
		assert_eq!(project_files.len(), 1);
		assert!(project_files[0].content.contains("Test context"));
	}

	#[test]
	fn gather_finds_dotclaude_claude_md() {
		let dir = tempfile::tempdir().unwrap();
		let dotclaude = dir.path().join(".claude");
		std::fs::create_dir(&dotclaude).unwrap();
		std::fs::write(dotclaude.join("CLAUDE.md"), "dotclaude content").unwrap();

		let result = gather(dir.path());
		let project_files: Vec<_> = result
			.iter()
			.filter(|f| f.path.contains(dir.path().to_str().unwrap()))
			.collect();
		assert_eq!(project_files.len(), 1);
		assert!(project_files[0].content.contains("dotclaude content"));
	}

	#[test]
	fn gather_skips_empty_files() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("CLAUDE.md"), "   \n  \n  ").unwrap();

		let result = gather(dir.path());
		let project_files: Vec<_> = result
			.iter()
			.filter(|f| f.path.contains(dir.path().to_str().unwrap()))
			.collect();
		assert!(project_files.is_empty(), "should skip whitespace-only files");
	}

	#[test]
	fn load_system_prompt_customization_does_not_crash() {
		let _ = load_system_prompt_customization();
	}
}
