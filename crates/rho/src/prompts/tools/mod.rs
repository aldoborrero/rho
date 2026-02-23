/// Get the rich description for a tool by name.
///
/// Returns the embedded `.md` content, or `None` if no rich description exists.
#[must_use]
pub fn tool_description(name: &str) -> Option<&'static str> {
	match name {
		"bash" => Some(include_str!("bash.md")),
		"read" => Some(include_str!("read.md")),
		"write" => Some(include_str!("write.md")),
		"grep" => Some(include_str!("grep.md")),
		"find" => Some(include_str!("find.md")),
		"clipboard" => Some(include_str!("clipboard.md")),
		"image" => Some(include_str!("image.md")),
		"process" => Some(include_str!("process.md")),
		"fuzzy_find" => Some(include_str!("fuzzy_find.md")),
		"html_to_markdown" => Some(include_str!("html_to_markdown.md")),
		"workmux" => Some(include_str!("workmux.md")),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn known_tools_have_descriptions() {
		let tools = [
			"bash",
			"read",
			"write",
			"grep",
			"find",
			"clipboard",
			"image",
			"process",
			"fuzzy_find",
			"html_to_markdown",
			"workmux",
		];
		for name in tools {
			assert!(tool_description(name).is_some(), "missing description for tool: {name}");
		}
	}

	#[test]
	fn unknown_tool_returns_none() {
		assert!(tool_description("nonexistent_tool").is_none());
	}

	#[test]
	fn descriptions_are_not_empty() {
		for name in ["bash", "read", "write", "grep", "find"] {
			let desc = tool_description(name).unwrap();
			assert!(!desc.trim().is_empty(), "empty description for: {name}");
		}
	}
}
