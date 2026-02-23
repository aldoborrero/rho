//! Chat message rendering component -- displays conversation history as styled
//! ANSI text.

use std::{collections::HashMap, rc::Rc};

use rho_tui::{
	component::{Component, InputResult},
	components::markdown::Markdown,
	symbols::SymbolTheme,
	theme::{Theme, ThemeBg, ThemeColor},
};

use crate::{
	ai::types::{AssistantMessage, ContentBlock, Message, ToolResultMessage, UserMessage},
	tui::tool_renderers::{ToolResultDisplay, get_tool_renderer},
};

/// Cached rendered output for a single tool result.
struct CachedRender {
	expanded: bool,
	width:    u16,
	lines:    Vec<String>,
}

/// Renders the conversation history as styled ANSI lines.
pub struct ChatComponent {
	messages:           Vec<Message>,
	/// Currently streaming text (appended to display but not yet committed).
	streaming_text:     String,
	/// Currently streaming thinking text.
	streaming_thinking: String,
	/// Whether we are currently streaming.
	is_streaming:       bool,
	/// Scroll offset (lines from bottom).
	scroll_offset:      usize,
	/// Theme for consistent styling.
	theme:              Rc<Theme>,
	/// Symbol theme for markdown rendering.
	symbols:            SymbolTheme,
	/// Whether tool output blocks are expanded (Ctrl+O toggle).
	tools_expanded:     bool,
	/// Side-table cache for rendered tool results, keyed by `tool_use_id`.
	render_cache:       HashMap<String, CachedRender>,
}

impl ChatComponent {
	pub fn new(theme: Rc<Theme>, symbols: SymbolTheme) -> Self {
		Self {
			messages: Vec::new(),
			streaming_text: String::new(),
			streaming_thinking: String::new(),
			is_streaming: false,
			scroll_offset: 0,
			theme,
			symbols,
			tools_expanded: false,
			render_cache: HashMap::new(),
		}
	}

	pub fn add_message(&mut self, message: Message) {
		self.messages.push(message);
	}

	pub fn start_streaming(&mut self) {
		self.is_streaming = true;
		self.streaming_text.clear();
		self.streaming_thinking.clear();
	}

	pub fn append_text(&mut self, text: &str) {
		self.streaming_text.push_str(text);
	}

	pub fn append_thinking(&mut self, text: &str) {
		self.streaming_thinking.push_str(text);
	}

	pub fn finish_streaming(&mut self) {
		self.is_streaming = false;
		self.streaming_text.clear();
		self.streaming_thinking.clear();
	}

	pub fn clear(&mut self) {
		self.messages.clear();
		self.streaming_text.clear();
		self.streaming_thinking.clear();
		self.scroll_offset = 0;
		self.render_cache.clear();
	}

	/// Toggle expanded/collapsed state for all tool output blocks (Ctrl+O).
	pub fn toggle_tool_expansion(&mut self) {
		self.tools_expanded = !self.tools_expanded;
		self.render_cache.clear();
	}

	/// Look up the tool name for a given `tool_use_id` by scanning assistant
	/// messages.
	fn find_tool_name(&self, tool_use_id: &str) -> Option<&str> {
		for msg in self.messages.iter().rev() {
			if let Message::Assistant(a) = msg {
				for block in &a.content {
					if let ContentBlock::ToolUse { id, name, .. } = block
						&& id == tool_use_id
					{
						return Some(name.as_str());
					}
				}
			}
		}
		None
	}

	fn render_user_message(&self, msg: &UserMessage, width: u16) -> Vec<String> {
		let mut lines = Vec::new();
		lines.push(String::new()); // blank separator
		for line in msg.content.lines() {
			let styled = self.theme.fg(ThemeColor::UserMessageText, line);
			// Pad to full width so background covers the entire line
			let vis_len = rho_text::width::visible_width_str(&styled);
			let padding = (width as usize).saturating_sub(vis_len);
			let padded = if padding > 0 {
				format!("{styled}{}", " ".repeat(padding))
			} else {
				styled
			};
			lines.push(self.theme.bg(ThemeBg::UserMessageBg, &padded));
		}
		lines
	}

	fn render_assistant_message(&self, msg: &AssistantMessage, width: u16) -> Vec<String> {
		let mut lines = Vec::new();
		lines.push(String::new()); // blank separator
		for block in &msg.content {
			match block {
				ContentBlock::Text { text } => {
					let md_theme = self.theme.markdown_theme(self.symbols.clone());
					let mut md = Markdown::new(text, 1, 0, md_theme, None, 2);
					let rendered = md.render(width);
					lines.extend(rendered);
				},
				ContentBlock::Thinking { thinking } => {
					let label = self
						.theme
						.fg(ThemeColor::ThinkingText, &self.theme.italic("[thinking]"));
					lines.push(format!("  {label}"));
					let thinking_lines: Vec<&str> = thinking.lines().collect();
					for line in thinking_lines.iter().take(5) {
						let styled = self
							.theme
							.fg(ThemeColor::ThinkingText, &self.theme.italic(line));
						lines.push(format!("  {styled}"));
					}
					if thinking_lines.len() > 5 {
						let more = format!("... ({} more lines)", thinking_lines.len() - 5);
						let styled = self
							.theme
							.fg(ThemeColor::ThinkingText, &self.theme.italic(&more));
						lines.push(format!("  {styled}"));
					}
				},
				ContentBlock::ToolUse { name, input, .. } => {
					let renderer = get_tool_renderer(name);
					lines.extend(renderer.render_call(input, &self.theme, width));
				},
			}
		}
		lines
	}

	fn render_tool_result(&mut self, msg: &ToolResultMessage, width: u16) -> Vec<String> {
		// Check cache first
		if let Some(cached) = self.render_cache.get(&msg.tool_use_id)
			&& cached.expanded == self.tools_expanded
			&& cached.width == width
		{
			return cached.lines.clone();
		}

		// Cache miss — render fresh
		let tool_name = self.find_tool_name(&msg.tool_use_id).unwrap_or("Unknown");
		let renderer = get_tool_renderer(tool_name);
		let display = ToolResultDisplay { content: msg.content.clone(), is_error: msg.is_error };
		let lines = renderer.render_result(&display, self.tools_expanded, &self.theme, width);

		// Store in cache
		self.render_cache.insert(msg.tool_use_id.clone(), CachedRender {
			expanded: self.tools_expanded,
			width,
			lines: lines.clone(),
		});

		lines
	}
}

impl Component for ChatComponent {
	fn render(&mut self, width: u16) -> Vec<String> {
		let mut lines = Vec::new();

		// Clone messages to avoid borrow conflict with &mut self in render_tool_result.
		let messages: Vec<Message> = self.messages.clone();
		for msg in &messages {
			match msg {
				Message::User(u) => lines.extend(self.render_user_message(u, width)),
				Message::Assistant(a) => lines.extend(self.render_assistant_message(a, width)),
				Message::ToolResult(t) => lines.extend(self.render_tool_result(t, width)),
			}
		}

		// Render streaming content
		if self.is_streaming {
			if !self.streaming_thinking.is_empty() {
				let label = self
					.theme
					.fg(ThemeColor::ThinkingText, &self.theme.italic("[thinking...]"));
				lines.push(format!("  {label}"));
			}
			if !self.streaming_text.is_empty() {
				lines.push(String::new());
				let md_theme = self.theme.markdown_theme(self.symbols.clone());
				let mut md = Markdown::new(&self.streaming_text, 1, 0, md_theme, None, 2);
				let rendered = md.render(width);
				lines.extend(rendered);
			}
		}

		lines
	}

	fn handle_input(&mut self, _data: &str) -> InputResult {
		InputResult::Ignored
	}
}

#[cfg(test)]
mod tests {
	use rho_tui::theme::ColorMode;

	use super::*;

	fn test_theme() -> Rc<Theme> {
		Rc::new(Theme::dark_with_mode(ColorMode::TrueColor))
	}

	fn test_symbols() -> SymbolTheme {
		SymbolTheme {
			cursor:         ">",
			input_cursor:   "|",
			box_round:      rho_tui::symbols::RoundedBoxSymbols {
				top_left:     "\u{256d}",
				top_right:    "\u{256e}",
				bottom_left:  "\u{2570}",
				bottom_right: "\u{256f}",
				horizontal:   "\u{2500}",
				vertical:     "\u{2502}",
			},
			box_sharp:      rho_tui::symbols::BoxSymbols {
				top_left:     "\u{250c}",
				top_right:    "\u{2510}",
				bottom_left:  "\u{2514}",
				bottom_right: "\u{2518}",
				horizontal:   "\u{2500}",
				vertical:     "\u{2502}",
				tee_down:     "\u{252c}",
				tee_up:       "\u{2534}",
				tee_left:     "\u{2524}",
				tee_right:    "\u{251c}",
				cross:        "\u{253c}",
			},
			table:          rho_tui::symbols::BoxSymbols {
				top_left:     "\u{250c}",
				top_right:    "\u{2510}",
				bottom_left:  "\u{2514}",
				bottom_right: "\u{2518}",
				horizontal:   "\u{2500}",
				vertical:     "\u{2502}",
				tee_down:     "\u{252c}",
				tee_up:       "\u{2534}",
				tee_left:     "\u{2524}",
				tee_right:    "\u{251c}",
				cross:        "\u{253c}",
			},
			quote_border:   "\u{2502}",
			hr_char:        "\u{2500}",
			spinner_frames: &["\u{280b}"],
		}
	}

	/// Helper: add an assistant message with a single ToolUse block.
	fn add_tool_use(
		chat: &mut ChatComponent,
		tool_use_id: &str,
		name: &str,
		input: serde_json::Value,
	) {
		chat.add_message(Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id: tool_use_id.to_owned(),
				name: name.to_owned(),
				input,
			}],
			stop_reason: None,
			usage:       None,
		}));
	}

	#[test]
	fn test_empty_chat_renders_empty() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		let lines = chat.render(80);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_user_message_renders() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.add_message(Message::User(UserMessage { content: "Hello".to_owned() }));
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("Hello")));
	}

	#[test]
	fn test_assistant_message_renders() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.add_message(Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::Text { text: "Hi there".to_owned() }],
			stop_reason: None,
			usage:       None,
		}));
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("Hi there")));
	}

	#[test]
	fn test_streaming_text() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.start_streaming();
		chat.append_text("Streaming...");
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("Streaming...")));
		chat.finish_streaming();
	}

	#[test]
	fn test_streaming_thinking() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.start_streaming();
		chat.append_thinking("Deep thought...");
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("thinking...")));
		chat.finish_streaming();
		let lines = chat.render(80);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_tool_use_renders_via_renderer() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.add_message(Message::Assistant(AssistantMessage {
			content:     vec![ContentBlock::ToolUse {
				id:    "tu_1".to_owned(),
				name:  "Bash".to_owned(),
				input: serde_json::json!({ "command": "ls -la" }),
			}],
			stop_reason: None,
			usage:       None,
		}));
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("Bash")),
			"tool use should render via BashRenderer with 'Bash' in output",
		);
	}

	#[test]
	fn test_tool_result_renders_via_renderer() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		// Need a preceding ToolUse for name lookup
		add_tool_use(&mut chat, "tu_1", "Bash", serde_json::json!({ "command": "ls" }));
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_1".to_owned(),
			content:     "file contents here".to_owned(),
			is_error:    false,
		}));
		let lines = chat.render(80);
		// Renderer produces bordered output with tool name, not old "[result]" format
		assert!(
			lines.iter().any(|l| l.contains("Bash")),
			"tool result should render via BashRenderer with 'Bash' in output",
		);
		assert!(
			lines.iter().any(|l| l.contains("file contents here")),
			"tool result should include content",
		);
	}

	#[test]
	fn test_tool_result_error_renders_via_renderer() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		add_tool_use(&mut chat, "tu_2", "Bash", serde_json::json!({ "command": "bad" }));
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_2".to_owned(),
			content:     "command failed".to_owned(),
			is_error:    true,
		}));
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("Bash")),
			"error tool result should render via BashRenderer",
		);
		assert!(
			lines.iter().any(|l| l.contains("\u{2718}")),
			"error tool result should contain cross mark",
		);
	}

	#[test]
	fn test_tool_result_unknown_tool_fallback() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		// No preceding ToolUse — falls back to "Unknown"
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_orphan".to_owned(),
			content:     "some output".to_owned(),
			is_error:    false,
		}));
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("Unknown")),
			"orphan tool result should render via DefaultRenderer('Unknown')",
		);
	}

	#[test]
	fn test_clear_resets_state() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.add_message(Message::User(UserMessage { content: "Hello".to_owned() }));
		chat.start_streaming();
		chat.append_text("streaming");
		chat.clear();
		let lines = chat.render(80);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_handle_input_ignored() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		assert_eq!(chat.handle_input("x"), InputResult::Ignored);
	}

	#[test]
	fn test_toggle_tool_expansion() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		assert!(!chat.tools_expanded);
		chat.toggle_tool_expansion();
		assert!(chat.tools_expanded);
		chat.toggle_tool_expansion();
		assert!(!chat.tools_expanded);
	}

	#[test]
	fn test_render_cache_invalidated_on_toggle() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		add_tool_use(&mut chat, "tu_1", "Bash", serde_json::json!({ "command": "echo hi" }));
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_1".to_owned(),
			content:     "hi".to_owned(),
			is_error:    false,
		}));
		// First render populates cache
		let _ = chat.render(80);
		assert!(!chat.render_cache.is_empty());
		// Toggle clears cache
		chat.toggle_tool_expansion();
		assert!(chat.render_cache.is_empty());
	}
}
