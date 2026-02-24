//! Chat message rendering component -- displays conversation history as styled
//! ANSI text.

use std::{collections::HashMap, rc::Rc};

use rho_tui::{
	component::{Component, InputResult},
	components::{
		loader::Loader,
		markdown::Markdown,
		output_block::{OutputBlockOptions, OutputBlockState, OutputSection, render_output_block},
	},
	symbols::SymbolTheme,
	theme::{Theme, ThemeBg, ThemeColor},
};

use crate::{
	ai::types::{AssistantMessage, ContentBlock, Message, ToolResultMessage, UserMessage},
	tui::tool_renderers::{
		ToolResultDisplay, collapse_lines, get_tool_renderer, make_bg_style, make_border_style,
	},
};

/// Cached rendered output for a single tool result.
struct CachedRender {
	expanded: bool,
	width:    u16,
	lines:    Vec<String>,
}

/// A bang command result (not a Message — purely for display).
struct BangOutput {
	command:  String,
	output:   String,
	is_error: bool,
}

/// Items in the chat display (messages or bang command outputs).
enum ChatItem {
	Message(Message),
	Bang(BangOutput),
}

/// Renders the conversation history as styled ANSI lines.
pub struct ChatComponent {
	items:              Vec<ChatItem>,
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
	/// Animated spinner for loading states.
	loader:             Loader,
	/// Currently executing tool name (for spinner message).
	tool_executing:     Option<String>,
	/// Currently streaming bang output (in-progress command).
	streaming_bang:     Option<BangOutput>,
}

impl ChatComponent {
	pub fn new(theme: Rc<Theme>, symbols: SymbolTheme) -> Self {
		let mut loader = Loader::new(
			Box::new({
				let theme = theme.clone();
				move |s: &str| theme.fg(ThemeColor::Accent, s)
			}),
			Box::new({
				let theme = theme.clone();
				move |s: &str| theme.fg(ThemeColor::Dim, s)
			}),
			"Thinking...",
		);
		loader.stop(); // Don't run until streaming starts.
		Self {
			items: Vec::new(),
			streaming_text: String::new(),
			streaming_thinking: String::new(),
			is_streaming: false,
			scroll_offset: 0,
			theme,
			symbols,
			tools_expanded: false,
			render_cache: HashMap::new(),
			loader,
			tool_executing: None,
			streaming_bang: None,
		}
	}

	pub fn add_message(&mut self, message: Message) {
		self.items.push(ChatItem::Message(message));
	}

	/// Add a completed bang command output to the chat display.
	pub fn add_bang_output(&mut self, command: &str, output: &str, is_error: bool) {
		self.items.push(ChatItem::Bang(BangOutput {
			command: command.to_owned(),
			output: output.to_owned(),
			is_error,
		}));
	}

	/// Start a streaming bang command output block.
	pub fn start_bang(&mut self, command: &str) {
		self.streaming_bang = Some(BangOutput {
			command:  command.to_owned(),
			output:   String::new(),
			is_error: false,
		});
		self.loader.set_message(&format!("$ {command}"));
		self.loader.start();
	}

	/// Append a chunk of output to the in-progress bang command.
	pub fn append_bang_output(&mut self, chunk: &str) {
		if let Some(ref mut bang) = self.streaming_bang {
			bang.output.push_str(chunk);
		}
	}

	/// Finish the streaming bang command and commit it to the display.
	pub fn finish_bang(&mut self, is_error: bool) {
		if let Some(mut bang) = self.streaming_bang.take() {
			bang.is_error = is_error;
			self.items.push(ChatItem::Bang(bang));
			self.loader.stop();
		}
	}

	pub fn start_streaming(&mut self) {
		self.is_streaming = true;
		self.streaming_text.clear();
		self.streaming_thinking.clear();
		self.loader.set_message("Thinking...");
		self.loader.start();
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
		self.loader.stop();
		self.tool_executing = None;
	}

	pub fn clear(&mut self) {
		self.items.clear();
		self.streaming_text.clear();
		self.streaming_thinking.clear();
		self.is_streaming = false;
		self.scroll_offset = 0;
		self.render_cache.clear();
		self.loader.stop();
		self.tool_executing = None;
		self.streaming_bang = None;
	}

	/// Toggle expanded/collapsed state for all tool output blocks (Ctrl+O).
	pub fn toggle_tool_expansion(&mut self) {
		self.tools_expanded = !self.tools_expanded;
		self.render_cache.clear();
	}

	/// Advance spinner animation. Returns true if re-render needed.
	pub fn tick(&mut self) -> bool {
		self.loader.tick()
	}

	/// Set the currently executing tool (shows spinner with tool name).
	pub fn set_tool_executing(&mut self, name: Option<String>) {
		match name {
			Some(ref n) => {
				self.loader.set_message(&format!("Running {n}..."));
				self.loader.start();
			},
			None => {
				// Reset to "Thinking..." for the next LLM turn.
				self.loader.set_message("Thinking...");
			},
		}
		self.tool_executing = name;
	}

	/// Look up the tool name and input args for a given `tool_use_id` by
	/// scanning assistant messages.
	fn find_tool_use_data(&self, tool_use_id: &str) -> Option<(&str, &serde_json::Value)> {
		for item in self.items.iter().rev() {
			if let ChatItem::Message(Message::Assistant(a)) = item {
				for block in &a.content {
					if let ContentBlock::ToolUse { id, name, input } = block
						&& id == tool_use_id
					{
						return Some((name.as_str(), input));
					}
				}
			}
		}
		None
	}

	/// Check whether a `ToolResult` message exists for the given `tool_use_id`.
	fn has_tool_result(&self, tool_use_id: &str) -> bool {
		self.items.iter().any(|item| {
			matches!(item, ChatItem::Message(Message::ToolResult(t)) if t.tool_use_id == tool_use_id)
		})
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
				ContentBlock::ToolUse { id, name, input } => {
					// Skip rendering the call block if a matching result already
					// exists — the combined block will be rendered from the result
					// side instead.
					if !self.has_tool_result(id) {
						let renderer = get_tool_renderer(name);
						lines.extend(renderer.render_call(input, &self.theme, width));
					}
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

		// Cache miss — render fresh.
		// Look up both name and args so we can render a combined block.
		let (tool_name, args) = self.find_tool_use_data(&msg.tool_use_id).map_or_else(
			|| ("Unknown".to_owned(), serde_json::Value::Null),
			|(n, a)| (n.to_owned(), a.clone()),
		);
		let renderer = get_tool_renderer(&tool_name);
		let display = ToolResultDisplay { content: msg.content.clone(), is_error: msg.is_error };
		let lines =
			renderer.render_combined(&args, &display, self.tools_expanded, &self.theme, width);

		// Store in cache
		self
			.render_cache
			.insert(msg.tool_use_id.clone(), CachedRender {
				expanded: self.tools_expanded,
				width,
				lines: lines.clone(),
			});

		lines
	}

	fn render_bang_output(&self, bang: &BangOutput, width: u16) -> Vec<String> {
		let state = if bang.is_error {
			OutputBlockState::Error
		} else {
			OutputBlockState::Success
		};
		let icon = if bang.is_error {
			"\u{2718}"
		} else {
			"\u{2714}"
		};
		let header_text = self
			.theme
			.fg(ThemeColor::ToolTitle, &self.theme.bold(&format!("{icon} Bash")));
		let header_width = rho_text::width::visible_width_str(&header_text);

		let max_lines = if self.tools_expanded { 50 } else { 20 };
		let content_lines: Vec<&str> = bang.output.lines().collect();
		let collapsed = collapse_lines(&content_lines, max_lines, &self.theme);

		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections: vec![
				OutputSection {
					label: Some(self.theme.dim("Command")),
					lines: vec![format!("$ {}", bang.command)],
				},
				OutputSection {
					label: Some(self.theme.dim("Output")),
					lines: collapsed,
				},
			],
			border_style: make_border_style(&self.theme, state),
			bg_style: make_bg_style(&self.theme, state),
		};
		let mut lines = Vec::new();
		lines.push(String::new()); // blank separator
		lines.extend(render_output_block(&opts, width));
		lines
	}

	fn render_bang_running(&mut self, bang: &BangOutput, width: u16) -> Vec<String> {
		let state = OutputBlockState::Running;
		let header_text = self
			.theme
			.fg(ThemeColor::ToolTitle, &self.theme.bold("\u{2b22} Bash"));
		let header_width = rho_text::width::visible_width_str(&header_text);

		let mut sections = vec![OutputSection {
			label: Some(self.theme.dim("Command")),
			lines: vec![format!("$ {}", bang.command)],
		}];
		if !bang.output.is_empty() {
			let max_lines = if self.tools_expanded { 50 } else { 20 };
			let content_lines: Vec<&str> = bang.output.lines().collect();
			let collapsed = collapse_lines(&content_lines, max_lines, &self.theme);
			sections.push(OutputSection {
				label: Some(self.theme.dim("Output")),
				lines: collapsed,
			});
		}

		let opts = OutputBlockOptions {
			header: header_text,
			header_width,
			state,
			sections,
			border_style: make_border_style(&self.theme, state),
			bg_style: make_bg_style(&self.theme, state),
		};
		let mut lines = Vec::new();
		lines.push(String::new()); // blank separator
		lines.extend(render_output_block(&opts, width));
		// Show spinner below the block
		lines.extend(self.loader.render(width));
		lines
	}
}

impl Component for ChatComponent {
	fn render(&mut self, width: u16) -> Vec<String> {
		let mut lines = Vec::new();

		// Clone items to avoid borrow conflict with &mut self in render_tool_result.
		// We need to iterate over items but render_tool_result needs &mut self for
		// cache.
		let item_count = self.items.len();
		for i in 0..item_count {
			match &self.items[i] {
				ChatItem::Message(Message::User(u)) => {
					let u = u.clone();
					lines.extend(self.render_user_message(&u, width));
				},
				ChatItem::Message(Message::Assistant(a)) => {
					let a = a.clone();
					lines.extend(self.render_assistant_message(&a, width));
				},
				ChatItem::Message(Message::ToolResult(t)) => {
					let t = t.clone();
					lines.extend(self.render_tool_result(&t, width));
				},
				ChatItem::Bang(bang) => {
					// BangOutput doesn't need &mut self, just render directly.
					// But we need to avoid the borrow conflict, so clone.
					let bang_lines = self.render_bang_output(
						&BangOutput {
							command:  bang.command.clone(),
							output:   bang.output.clone(),
							is_error: bang.is_error,
						},
						width,
					);
					lines.extend(bang_lines);
				},
			}
		}

		// Render in-progress bang output block
		if let Some(ref bang) = self.streaming_bang {
			let bang_clone = BangOutput {
				command:  bang.command.clone(),
				output:   bang.output.clone(),
				is_error: bang.is_error,
			};
			lines.extend(self.render_bang_running(&bang_clone, width));
		}

		// Render streaming content
		if self.is_streaming {
			let has_content = !self.streaming_text.is_empty() || !self.streaming_thinking.is_empty();
			if !has_content && self.tool_executing.is_none() {
				// No content yet — show spinner
				lines.extend(self.loader.render(width));
			} else {
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
				// Show tool execution spinner after streaming text
				if self.tool_executing.is_some() {
					lines.extend(self.loader.render(width));
				}
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

	#[test]
	fn test_bang_output_renders() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.add_bang_output("ls -la", "total 42\ndrwxr-xr-x", false);
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("Bash")), "bang output should show Bash title");
		assert!(
			lines.iter().any(|l| l.contains("$ ls -la")),
			"bang output should show command in section",
		);
		assert!(
			lines.iter().any(|l| l.contains("\u{2714}")),
			"successful bang output should show check mark",
		);
	}

	#[test]
	fn test_bang_output_error_renders() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.add_bang_output("false", "command failed", true);
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("Bash")), "error bang output should show Bash title");
		assert!(
			lines.iter().any(|l| l.contains("$ false")),
			"error bang output should show command in section",
		);
		assert!(
			lines.iter().any(|l| l.contains("\u{2718}")),
			"error bang output should show cross mark",
		);
	}

	#[test]
	fn test_streaming_bang_renders() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.start_bang("sleep 3");
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("sleep 3")),
			"streaming bang should show the command",
		);
		chat.append_bang_output("chunk1\n");
		chat.append_bang_output("chunk2\n");
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("chunk1")),
			"streaming bang should show accumulated output",
		);
		chat.finish_bang(false);
		let lines = chat.render(80);
		assert!(lines.iter().any(|l| l.contains("\u{2714}")), "finished bang should show check mark",);
	}

	#[test]
	fn test_spinner_shows_when_streaming_no_content() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		chat.start_streaming();
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("Thinking...")),
			"spinner should show 'Thinking...' when streaming with no content",
		);
		chat.finish_streaming();
	}

	// ── Combined rendering ─────────────────────────────────────────

	#[test]
	fn test_tool_use_suppressed_when_result_exists() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		add_tool_use(&mut chat, "tu_1", "Bash", serde_json::json!({ "command": "ls" }));
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_1".to_owned(),
			content:     "file.txt".to_owned(),
			is_error:    false,
		}));
		let lines = chat.render(80);
		// The running-state header (⬢) should NOT appear
		let running_blocks = lines.iter().filter(|l| l.contains('\u{2b22}')).count();
		assert_eq!(
			running_blocks, 0,
			"ToolUse running block should be suppressed when result exists",
		);
		// The combined result block (✔) should appear
		assert!(
			lines.iter().any(|l| l.contains('\u{2714}')),
			"combined result block should appear with check mark",
		);
	}

	#[test]
	fn test_tool_use_shown_when_running() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		// Only ToolUse, no ToolResult yet — should show running block
		add_tool_use(&mut chat, "tu_1", "Bash", serde_json::json!({ "command": "sleep 5" }));
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains('\u{2b22}')),
			"ToolUse should render running block when no result exists",
		);
	}

	#[test]
	fn test_combined_block_shows_command_and_output() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		add_tool_use(&mut chat, "tu_1", "Bash", serde_json::json!({ "command": "echo hello" }));
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_1".to_owned(),
			content:     "hello".to_owned(),
			is_error:    false,
		}));
		let lines = chat.render(80);
		assert!(
			lines.iter().any(|l| l.contains("$ echo hello")),
			"combined block should include the command",
		);
		assert!(
			lines.iter().any(|l| l.contains("hello")),
			"combined block should include the output",
		);
	}

	#[test]
	fn test_parallel_tools_one_running_one_done() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		// Assistant message with two tool uses
		chat.add_message(Message::Assistant(AssistantMessage {
			content:     vec![
				ContentBlock::ToolUse {
					id:    "tu_a".to_owned(),
					name:  "Bash".to_owned(),
					input: serde_json::json!({ "command": "ls" }),
				},
				ContentBlock::ToolUse {
					id:    "tu_b".to_owned(),
					name:  "Grep".to_owned(),
					input: serde_json::json!({ "pattern": "TODO" }),
				},
			],
			stop_reason: None,
			usage:       None,
		}));
		// Only first tool has a result
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_a".to_owned(),
			content:     "file.txt".to_owned(),
			is_error:    false,
		}));
		let lines = chat.render(80);
		// tu_a: suppressed ToolUse, combined result shown
		assert!(
			lines.iter().any(|l| l.contains('\u{2714}')),
			"completed tool should show combined result with check mark",
		);
		// tu_b: still running, should show running block
		assert!(
			lines.iter().any(|l| l.contains('\u{2b22}')),
			"running tool should still show running block",
		);
	}

	#[test]
	fn test_has_tool_result() {
		let mut chat = ChatComponent::new(test_theme(), test_symbols());
		add_tool_use(&mut chat, "tu_1", "Bash", serde_json::json!({}));
		assert!(!chat.has_tool_result("tu_1"), "no result yet");
		chat.add_message(Message::ToolResult(ToolResultMessage {
			tool_use_id: "tu_1".to_owned(),
			content:     "ok".to_owned(),
			is_error:    false,
		}));
		assert!(chat.has_tool_result("tu_1"), "result exists");
		assert!(!chat.has_tool_result("tu_2"), "different id has no result");
	}
}
