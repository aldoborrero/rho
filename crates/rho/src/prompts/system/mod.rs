use minijinja::Environment;

use super::types::PromptContext;

static TEMPLATE: &str = include_str!("system-prompt.md");

/// Render the system prompt template with the given context.
pub fn render(ctx: &PromptContext) -> anyhow::Result<String> {
	let mut env = Environment::new();
	env.add_template("system-prompt", TEMPLATE)?;
	let tmpl = env.get_template("system-prompt")?;
	let raw = tmpl.render(ctx)?;
	Ok(optimize_layout(&raw))
}

/// Post-process rendered template output.
///
/// - Normalize CRLF to LF
/// - Trim trailing whitespace per line
/// - Collapse runs of 2+ blank lines into exactly 1 blank line
/// - Trim leading/trailing whitespace of the whole output
fn optimize_layout(input: &str) -> String {
	let normalized = input.replace("\r\n", "\n");
	let mut result = String::with_capacity(normalized.len());
	let mut blank_count: u32 = 0;

	for line in normalized.lines() {
		let trimmed = line.trim_end();
		if trimmed.is_empty() {
			blank_count += 1;
			if blank_count <= 1 {
				result.push('\n');
			}
		} else {
			if blank_count > 0 && !result.is_empty() && !result.ends_with('\n') {
				result.push('\n');
			}
			blank_count = 0;
			result.push_str(trimmed);
			result.push('\n');
		}
	}

	result.trim().to_owned()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::prompts::types::*;

	fn minimal_context() -> PromptContext {
		PromptContext {
			tools: vec!["bash".into(), "read".into(), "write".into(), "grep".into(), "find".into()],
			tool_descriptions: vec![
				ToolDescription { name: "bash".into(), description: "Execute commands".into() },
				ToolDescription { name: "read".into(), description: "Read files".into() },
			],
			repeat_tool_descriptions: false,
			environment: vec![EnvItem { label: "OS".into(), value: "linux".into() }, EnvItem {
				label: "Arch".into(),
				value: "x86_64".into(),
			}],
			system_prompt_customization: None,
			context_files: vec![],
			git: None,
			date: "2026-02-23".into(),
			cwd: "/home/user/project".into(),
			append_system_prompt: None,
		}
	}

	#[test]
	fn template_renders_without_error() {
		let ctx = minimal_context();
		let result = render(&ctx);
		assert!(result.is_ok(), "render failed: {:?}", result.err());
	}

	#[test]
	fn template_contains_identity_section() {
		let result = render(&minimal_context()).unwrap();
		assert!(result.contains("<identity>"));
		assert!(result.contains("Distinguished Staff Engineer"));
	}

	#[test]
	fn template_contains_environment() {
		let result = render(&minimal_context()).unwrap();
		assert!(result.contains("OS: linux"));
		assert!(result.contains("Arch: x86_64"));
	}

	#[test]
	fn template_contains_tool_names() {
		let result = render(&minimal_context()).unwrap();
		assert!(result.contains("- bash"), "missing bash in tools list. Got:\n{result}");
		assert!(result.contains("- read"), "missing read in tools list. Got:\n{result}");
	}

	#[test]
	fn template_contains_date_and_cwd() {
		let result = render(&minimal_context()).unwrap();
		assert!(result.contains("2026-02-23"));
		assert!(result.contains("/home/user/project"));
	}

	#[test]
	fn template_includes_git_context_when_present() {
		let mut ctx = minimal_context();
		ctx.git = Some(GitContext {
			is_repo:        true,
			current_branch: "feat/test".into(),
			main_branch:    "main".into(),
			status:         "(clean)".into(),
			commits:        "abc1234 initial commit".into(),
		});
		let result = render(&ctx).unwrap();
		assert!(result.contains("feat/test"));
		assert!(result.contains("(clean)"));
		assert!(result.contains("abc1234"));
	}

	#[test]
	fn template_omits_git_section_when_absent() {
		let ctx = minimal_context();
		let result = render(&ctx).unwrap();
		assert!(!result.contains("Version Control"));
	}

	#[test]
	fn template_includes_context_files() {
		let mut ctx = minimal_context();
		ctx.context_files = vec![ContextFile {
			path:    "/home/user/.claude/CLAUDE.md".into(),
			content: "Always use tabs.".into(),
		}];
		let result = render(&ctx).unwrap();
		assert!(result.contains("Always use tabs."));
	}

	#[test]
	fn template_includes_system_customization() {
		let mut ctx = minimal_context();
		ctx.system_prompt_customization = Some("Custom system instructions here.".into());
		let result = render(&ctx).unwrap();
		assert!(result.contains("<context>"));
		assert!(result.contains("Custom system instructions here."));
	}

	#[test]
	fn template_omits_context_when_no_customization() {
		let ctx = minimal_context();
		let result = render(&ctx).unwrap();
		assert!(!result.contains("<context>"));
	}

	#[test]
	fn template_includes_append_prompt() {
		let mut ctx = minimal_context();
		ctx.append_system_prompt = Some("Extra instructions appended.".into());
		let result = render(&ctx).unwrap();
		assert!(result.contains("Extra instructions appended."));
	}

	#[test]
	fn template_tool_precedence_with_bash() {
		let ctx = minimal_context();
		let result = render(&ctx).unwrap();
		assert!(result.contains("Precedence"), "missing Precedence section. Got:\n{result}");
	}

	#[test]
	fn template_tool_precedence_without_bash() {
		let mut ctx = minimal_context();
		ctx.tools = vec!["read".into(), "write".into()];
		let result = render(&ctx).unwrap();
		assert!(!result.contains("Precedence"));
	}

	#[test]
	fn repeat_tool_descriptions_shows_full_descriptions() {
		let mut ctx = minimal_context();
		ctx.repeat_tool_descriptions = true;
		let result = render(&ctx).unwrap();
		assert!(result.contains("<tool name=\"bash\">"), "missing tool tag. Got:\n{result}");
		assert!(result.contains("Execute commands"));
	}

	#[test]
	fn optimize_layout_collapses_blank_lines() {
		let input = "line1\n\n\n\n\nline2";
		let result = optimize_layout(input);
		assert_eq!(result, "line1\n\nline2");
	}

	#[test]
	fn optimize_layout_trims_trailing_whitespace() {
		let input = "hello   \nworld  ";
		let result = optimize_layout(input);
		assert_eq!(result, "hello\nworld");
	}

	#[test]
	fn optimize_layout_normalizes_crlf() {
		let input = "hello\r\nworld\r\n";
		let result = optimize_layout(input);
		assert_eq!(result, "hello\nworld");
	}
}
