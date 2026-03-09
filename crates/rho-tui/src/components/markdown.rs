//! Markdown renderer — converts markdown text to ANSI-styled terminal lines.
//!
//! Uses `pulldown-cmark` for parsing (replaces the TypeScript `marked`
//! library). Supports headings, paragraphs, lists, tables, blockquotes, code
//! blocks with syntax highlighting, horizontal rules, inline styles, links, and
//! mermaid/image rendering.

use std::{
	cmp::{max, min},
	rc::Rc,
};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rho_text::width::visible_width_str;

use super::text::make_padding;
use crate::{
	capabilities::{CellDimensions, ImageProtocol, TerminalInfo, encode_iterm2, encode_kitty},
	component::Component,
	highlight::HighlightColors,
	symbols::SymbolTheme,
};

// ============================================================================
// Types
// ============================================================================

/// Style function: takes text, returns styled text with ANSI codes.
pub type StyleFn = Rc<dyn Fn(&str) -> String>;

/// Syntax highlighting function: (code, lang) -> highlighted lines.
pub type HighlightCodeFn = Option<Rc<dyn Fn(&str, Option<&str>) -> Vec<String>>>;

/// Mermaid image lookup function: source hash -> optional image data.
pub type GetMermaidImageFn = Option<Rc<dyn Fn(&str) -> Option<MermaidImage>>>;

/// Default text styling applied to all markdown content unless overridden.
#[derive(Clone)]
pub struct DefaultTextStyle {
	/// Foreground color function.
	pub color:         Option<StyleFn>,
	/// Background color function — applied at padding stage for full line width.
	pub bg_color:      Option<StyleFn>,
	/// Bold text.
	pub bold:          bool,
	/// Italic text.
	pub italic:        bool,
	/// Strikethrough text.
	pub strikethrough: bool,
	/// Underline text.
	pub underline:     bool,
}

/// Pre-rendered mermaid image data.
pub struct MermaidImage {
	pub base64:    String,
	pub width_px:  u32,
	pub height_px: u32,
}

/// Theme functions for styling markdown elements.
#[derive(Clone)]
pub struct MarkdownTheme {
	pub heading:           StyleFn,
	pub link:              StyleFn,
	pub link_url:          StyleFn,
	pub code:              StyleFn,
	pub code_block:        StyleFn,
	pub code_block_border: StyleFn,
	pub quote:             StyleFn,
	pub quote_border:      StyleFn,
	pub hr:                StyleFn,
	pub list_bullet:       StyleFn,
	pub bold:              StyleFn,
	pub italic:            StyleFn,
	pub strikethrough:     StyleFn,
	pub underline:         StyleFn,
	/// Optional syntax highlighting: (code, lang) -> highlighted lines.
	pub highlight_code:    HighlightCodeFn,
	/// Optional mermaid image lookup by source hash.
	pub get_mermaid_image: GetMermaidImageFn,
	pub symbols:           SymbolTheme,
	/// Highlight colors for code blocks (used with pi-highlight).
	pub highlight_colors:  Option<HighlightColors>,
}

// ============================================================================
// Markdown component
// ============================================================================

/// Markdown renderer component.
pub struct Markdown {
	text:                 String,
	padding_x:            usize,
	padding_y:            usize,
	theme:                MarkdownTheme,
	default_text_style:   Option<DefaultTextStyle>,
	code_block_indent:    usize,
	// Cache
	cached_text:          Option<String>,
	cached_width:         Option<u16>,
	cached_lines:         Option<Vec<String>>,
	// Lazily computed default style prefix
	default_style_prefix: Option<String>,
	// Terminal info for image rendering
	terminal_info:        Option<TerminalInfo>,
	cell_dims:            CellDimensions,
}

impl Markdown {
	pub fn new(
		text: &str,
		padding_x: usize,
		padding_y: usize,
		theme: MarkdownTheme,
		default_text_style: Option<DefaultTextStyle>,
		code_block_indent: usize,
	) -> Self {
		Self {
			text: text.to_owned(),
			padding_x,
			padding_y,
			theme,
			default_text_style,
			code_block_indent: max(0, code_block_indent),
			cached_text: None,
			cached_width: None,
			cached_lines: None,
			default_style_prefix: None,
			terminal_info: None,
			cell_dims: CellDimensions::default(),
		}
	}

	pub fn set_text(&mut self, text: &str) {
		text.clone_into(&mut self.text);
		self.invalidate();
	}

	pub const fn set_terminal_info(&mut self, info: TerminalInfo, cell_dims: CellDimensions) {
		self.terminal_info = Some(info);
		self.cell_dims = cell_dims;
	}

	fn invalidate(&mut self) {
		self.cached_text = None;
		self.cached_width = None;
		self.cached_lines = None;
		self.default_style_prefix = None;
	}

	// ── Default style helpers ───────────────────────────────────────

	fn apply_default_style(&self, text: &str) -> String {
		let Some(ref style) = self.default_text_style else {
			return text.to_owned();
		};

		let mut styled = text.to_owned();

		if let Some(ref color_fn) = style.color {
			styled = color_fn(&styled);
		}
		if style.bold {
			styled = (self.theme.bold)(&styled);
		}
		if style.italic {
			styled = (self.theme.italic)(&styled);
		}
		if style.strikethrough {
			styled = (self.theme.strikethrough)(&styled);
		}
		if style.underline {
			styled = (self.theme.underline)(&styled);
		}

		styled
	}

	fn get_default_style_prefix(&mut self) -> String {
		if let Some(ref prefix) = self.default_style_prefix {
			return prefix.clone();
		}

		let prefix = if self.default_text_style.is_some() {
			self.compute_style_prefix(|md, sentinel| md.apply_default_style(sentinel))
		} else {
			String::new()
		};

		self.default_style_prefix = Some(prefix.clone());
		prefix
	}

	fn compute_style_prefix(&self, style_fn: impl Fn(&Self, &str) -> String) -> String {
		let sentinel = "\u{0000}";
		let styled = style_fn(self, sentinel);
		if let Some(idx) = styled.find(sentinel) {
			styled[..idx].to_owned()
		} else {
			String::new()
		}
	}

	// ── Rendering pipeline ──────────────────────────────────────────

	fn render_impl(&mut self, width: u16) -> Vec<String> {
		let w = width as usize;
		let content_width = max(1, w.saturating_sub(self.padding_x * 2));

		// Empty text
		if self.text.is_empty() || self.text.trim().is_empty() {
			return Vec::new();
		}

		// Replace tabs with 3 spaces
		let normalized = self.text.replace('\t', "   ");

		// Parse markdown and render tokens
		let rendered_lines = self.render_tokens(&normalized, content_width);

		// Wrap lines
		let mut wrapped_lines = Vec::new();
		for line in &rendered_lines {
			if self.is_image_line(line) {
				wrapped_lines.push(line.clone());
			} else {
				let w = rho_text::wrap::wrap_text_with_ansi_str(line, content_width);
				wrapped_lines.extend(w);
			}
		}

		// Add margins and background
		let left_margin = make_padding(self.padding_x);
		let right_margin = make_padding(self.padding_x);
		let mut content_lines = Vec::new();

		for line in &wrapped_lines {
			if self.is_image_line(line) {
				content_lines.push(line.clone());
				continue;
			}

			let with_margins = format!("{left_margin}{line}{right_margin}");

			if let Some(ref style) = self.default_text_style
				&& let Some(ref bg_fn) = style.bg_color
			{
				content_lines.push(apply_background_to_line(&with_margins, w, &**bg_fn));
			} else {
				let vis_len = visible_width_str(&with_margins);
				let padding_needed = w.saturating_sub(vis_len);
				content_lines.push(format!("{with_margins}{}", make_padding(padding_needed)));
			}
		}

		// Add vertical padding
		let empty_line = make_padding(w);
		let mut result = Vec::new();
		for _ in 0..self.padding_y {
			if let Some(ref style) = self.default_text_style
				&& let Some(ref bg_fn) = style.bg_color
			{
				result.push(apply_background_to_line(&empty_line, w, &**bg_fn));
			} else {
				result.push(empty_line.clone());
			}
		}
		result.extend(content_lines);
		for _ in 0..self.padding_y {
			if let Some(ref style) = self.default_text_style
				&& let Some(ref bg_fn) = style.bg_color
			{
				result.push(apply_background_to_line(&empty_line, w, &**bg_fn));
			} else {
				result.push(empty_line.clone());
			}
		}

		if result.is_empty() {
			vec![String::new()]
		} else {
			result
		}
	}

	fn is_image_line(&self, line: &str) -> bool {
		if let Some(ref info) = self.terminal_info {
			info.is_image_line(line)
		} else {
			false
		}
	}

	// ── Token rendering ─────────────────────────────────────────────

	fn render_tokens(&mut self, text: &str, width: usize) -> Vec<String> {
		let opts = Options::ENABLE_TABLES
			| Options::ENABLE_STRIKETHROUGH
			| Options::ENABLE_HEADING_ATTRIBUTES;
		let parser = Parser::new_ext(text, opts);
		let events: Vec<Event<'_>> = parser.collect();

		let mut lines: Vec<String> = Vec::new();
		let mut i = 0;

		while i < events.len() {
			let consumed = self.render_block_event(&events, i, width, &mut lines);
			i += consumed;
		}

		lines
	}

	/// Render a top-level block event. Returns the number of events consumed.
	fn render_block_event(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		width: usize,
		lines: &mut Vec<String>,
	) -> usize {
		let event = &events[start];

		match event {
			Event::Start(Tag::Heading { level, .. }) => {
				self.render_heading(events, start, *level, lines)
			},
			Event::Start(Tag::Paragraph) => self.render_paragraph(events, start, width, lines),
			Event::Start(Tag::CodeBlock(kind)) => {
				let lang = match kind {
					CodeBlockKind::Fenced(lang) => {
						let l = lang.as_ref().trim();
						if l.is_empty() {
							None
						} else {
							Some(l.to_owned())
						}
					},
					CodeBlockKind::Indented => None,
				};
				self.render_code_block(events, start, lang.as_deref(), width, lines)
			},
			Event::Start(Tag::List(first_item)) => {
				self.render_list(events, start, *first_item, 0, lines)
			},
			Event::Start(Tag::Table(alignments)) => {
				let aligns = alignments.clone();
				self.render_table(events, start, &aligns, width, lines)
			},
			Event::Start(Tag::BlockQuote(..)) => self.render_blockquote(events, start, width, lines),
			Event::Rule => {
				let rule_width = min(width, 80);
				let hr_text = self.theme.symbols.hr_char.repeat(rule_width);
				lines.push((self.theme.hr)(&hr_text));
				Self::add_spacing_after(events, start, lines);
				1
			},
			Event::Start(Tag::HtmlBlock) => self.render_html_block(events, start, lines),
			Event::SoftBreak | Event::HardBreak => {
				lines.push(String::new());
				1
			},
			_ => {
				// Skip unknown events
				1
			},
		}
	}

	fn add_spacing_after(events: &[Event<'_>], end_pos: usize, lines: &mut Vec<String>) {
		// Look for next block start after this event
		let mut j = end_pos + 1;
		while j < events.len() {
			match &events[j] {
				Event::End(_) => {
					j += 1;
				},
				Event::SoftBreak | Event::HardBreak => return, // space follows, don't add extra
				_ => {
					lines.push(String::new());
					return;
				},
			}
		}
		// End of document — add spacing
		lines.push(String::new());
	}

	// ── Heading ─────────────────────────────────────────────────────

	fn render_heading(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		level: HeadingLevel,
		lines: &mut Vec<String>,
	) -> usize {
		let (inline_text, consumed) = self.collect_inline_text(events, start);
		let depth = heading_level_depth(level);

		let styled = if depth == 1 {
			(self.theme.heading)(&(self.theme.bold)(&(self.theme.underline)(&inline_text)))
		} else if depth == 2 {
			(self.theme.heading)(&(self.theme.bold)(&inline_text))
		} else {
			let prefix = format!("{} ", "#".repeat(depth));
			(self.theme.heading)(&(self.theme.bold)(&format!("{prefix}{inline_text}")))
		};

		lines.push(styled);

		// Add spacing after unless next is a blank event
		let end_pos = start + consumed - 1;
		Self::add_spacing_after(events, end_pos, lines);

		consumed
	}

	// ── Paragraph ───────────────────────────────────────────────────

	fn render_paragraph(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		_width: usize,
		lines: &mut Vec<String>,
	) -> usize {
		let (inline_text, consumed) = self.collect_inline_text(events, start);
		lines.push(inline_text);

		// Don't add spacing if next is list or space
		let end_pos = start + consumed - 1;
		if !Self::next_block_is_list_or_space(events, end_pos) {
			Self::add_spacing_after(events, end_pos, lines);
		}

		consumed
	}

	fn next_block_is_list_or_space(events: &[Event<'_>], end_pos: usize) -> bool {
		let mut j = end_pos + 1;
		while j < events.len() {
			match &events[j] {
				Event::End(_) => {
					j += 1;
				},
				Event::Start(Tag::List(..)) => return true,
				Event::SoftBreak | Event::HardBreak => return true,
				_ => return false,
			}
		}
		false
	}

	// ── Code block ──────────────────────────────────────────────────

	fn render_code_block(
		&self,
		events: &[Event<'_>],
		start: usize,
		lang: Option<&str>,
		width: usize,
		lines: &mut Vec<String>,
	) -> usize {
		// Collect code text
		let mut code = String::new();
		let mut i = start + 1;
		while i < events.len() {
			match &events[i] {
				Event::End(TagEnd::CodeBlock) => {
					i += 1;
					break;
				},
				Event::Text(text) => {
					code.push_str(text);
					i += 1;
				},
				_ => {
					i += 1;
				},
			}
		}
		let consumed = i - start;

		// Remove trailing newline from code
		if code.ends_with('\n') {
			code.pop();
		}

		// Handle mermaid diagrams
		if lang == Some("mermaid")
			&& let Some(ref get_mermaid) = self.theme.get_mermaid_image
		{
			let hash = simple_hash(code.trim());
			if let Some(image) = get_mermaid(&hash)
				&& let Some(image_lines) = self.render_mermaid_image(&image, width)
			{
				lines.extend(image_lines);
				let end_pos = start + consumed - 1;
				Self::add_spacing_after(events, end_pos, lines);
				return consumed;
			}
		}

		let code_indent = make_padding(self.code_block_indent);
		lines.push((self.theme.code_block_border)(&format!("```{}", lang.unwrap_or(""))));

		// Try syntax highlighting first
		if let Some(ref highlight_fn) = self.theme.highlight_code {
			let highlighted = highlight_fn(&code, lang);
			for hl_line in &highlighted {
				lines.push(format!("{code_indent}{hl_line}"));
			}
		} else if let Some(ref colors) = self.theme.highlight_colors
			&& lang.is_some()
		{
			// Use pi-highlight
			let highlighted = crate::highlight::highlight_code(&code, lang, colors);
			for hl_line in highlighted.split('\n') {
				lines.push(format!("{code_indent}{hl_line}"));
			}
		} else {
			for code_line in code.split('\n') {
				lines.push(format!("{code_indent}{}", (self.theme.code_block)(code_line)));
			}
		}

		lines.push((self.theme.code_block_border)("```"));

		let end_pos = start + consumed - 1;
		Self::add_spacing_after(events, end_pos, lines);

		consumed
	}

	// ── Lists ───────────────────────────────────────────────────────

	fn render_list(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		first_item: Option<u64>,
		depth: usize,
		lines: &mut Vec<String>,
	) -> usize {
		let indent = "  ".repeat(depth);
		let ordered = first_item.is_some();
		let start_number = first_item.unwrap_or(1);

		let mut i = start + 1; // skip Start(List)
		let mut item_index: u64 = 0;

		while i < events.len() {
			match &events[i] {
				Event::End(TagEnd::List(..)) => {
					i += 1;
					break;
				},
				Event::Start(Tag::Item) => {
					let bullet = if ordered {
						format!("{}. ", start_number + item_index)
					} else {
						"- ".to_owned()
					};
					item_index += 1;

					// Collect item lines
					let (item_lines, consumed) = self.render_list_item(events, i, depth);
					i += consumed;

					if item_lines.is_empty() {
						lines.push(format!("{}{}", indent, (self.theme.list_bullet)(&bullet)));
					} else {
						// First line
						let first = &item_lines[0];
						let is_nested = is_nested_list_line(first);

						if is_nested {
							lines.push(first.clone());
						} else {
							lines.push(format!(
								"{}{}{}",
								indent,
								(self.theme.list_bullet)(&bullet),
								first
							));
						}

						// Rest of the lines
						for item_line in &item_lines[1..] {
							if is_nested_list_line(item_line) {
								lines.push(item_line.clone());
							} else {
								lines.push(format!("{indent}  {item_line}"));
							}
						}
					}
				},
				_ => {
					i += 1;
				},
			}
		}

		i - start
	}

	/// Render list item tokens. Returns (lines, events consumed).
	fn render_list_item(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		parent_depth: usize,
	) -> (Vec<String>, usize) {
		let mut lines = Vec::new();
		let mut i = start + 1; // skip Start(Item)

		while i < events.len() {
			match &events[i] {
				Event::End(TagEnd::Item) => {
					i += 1;
					break;
				},
				Event::Start(Tag::List(first_item)) => {
					let first = *first_item;
					let consumed = self.render_list(events, i, first, parent_depth + 1, &mut lines);
					i += consumed;
				},
				Event::Start(Tag::Paragraph) => {
					let (text, consumed) = self.collect_inline_text(events, i);
					lines.push(text);
					i += consumed;
				},
				Event::Start(Tag::CodeBlock(kind)) => {
					let lang = match kind {
						CodeBlockKind::Fenced(lang) => {
							let l = lang.as_ref().trim();
							if l.is_empty() {
								None
							} else {
								Some(l.to_owned())
							}
						},
						CodeBlockKind::Indented => None,
					};
					let code_indent = make_padding(self.code_block_indent);

					// Collect code text
					let mut code = String::new();
					let mut j = i + 1;
					while j < events.len() {
						match &events[j] {
							Event::End(TagEnd::CodeBlock) => {
								j += 1;
								break;
							},
							Event::Text(text) => {
								code.push_str(text);
								j += 1;
							},
							_ => {
								j += 1;
							},
						}
					}
					if code.ends_with('\n') {
						code.pop();
					}

					lines.push((self.theme.code_block_border)(&format!(
						"```{}",
						lang.as_deref().unwrap_or("")
					)));
					if let Some(ref highlight_fn) = self.theme.highlight_code {
						let highlighted = highlight_fn(&code, lang.as_deref());
						for hl_line in &highlighted {
							lines.push(format!("{code_indent}{hl_line}"));
						}
					} else {
						for code_line in code.split('\n') {
							lines.push(format!("{code_indent}{}", (self.theme.code_block)(code_line)));
						}
					}
					lines.push((self.theme.code_block_border)("```"));

					i = j;
				},
				Event::Text(text) => {
					// Plain text in list item — may need inline rendering
					let rendered = self.apply_default_style(text);
					lines.push(rendered);
					i += 1;
				},
				_ => {
					i += 1;
				},
			}
		}

		(lines, i - start)
	}

	// ── Blockquote ──────────────────────────────────────────────────

	fn render_blockquote(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		width: usize,
		lines: &mut Vec<String>,
	) -> usize {
		// Collect blockquote content by rendering inner blocks
		let mut inner_lines = Vec::new();
		let mut i = start + 1;

		while i < events.len() {
			match &events[i] {
				Event::End(TagEnd::BlockQuote(..)) => {
					i += 1;
					break;
				},
				Event::Start(Tag::Paragraph) => {
					let (text, consumed) = self.collect_inline_text_with_style(events, i, true);
					inner_lines.push(text);
					i += consumed;
				},
				_ => {
					i += 1;
				},
			}
		}
		let consumed = i - start;

		// Wrap and add border
		let quote_content_width = max(1, width.saturating_sub(2));
		let border_char = self.theme.symbols.quote_border;

		for inner_line in &inner_lines {
			for sub_line in inner_line.split('\n') {
				let wrapped = rho_text::wrap::wrap_text_with_ansi_str(sub_line, quote_content_width);
				for wrapped_line in &wrapped {
					let border = (self.theme.quote_border)(&format!("{border_char} "));
					lines.push(format!("{border}{wrapped_line}"));
				}
			}
		}

		let end_pos = start + consumed - 1;
		Self::add_spacing_after(events, end_pos, lines);

		consumed
	}

	// ── HTML block ──────────────────────────────────────────────────

	fn render_html_block(
		&self,
		events: &[Event<'_>],
		start: usize,
		lines: &mut Vec<String>,
	) -> usize {
		let mut html = String::new();
		let mut i = start + 1;
		while i < events.len() {
			match &events[i] {
				Event::End(TagEnd::HtmlBlock) => {
					i += 1;
					break;
				},
				Event::Html(text) | Event::Text(text) => {
					html.push_str(text);
					i += 1;
				},
				_ => {
					i += 1;
				},
			}
		}
		if !html.trim().is_empty() {
			lines.push(self.apply_default_style(html.trim()));
		}
		i - start
	}

	// ── Table ───────────────────────────────────────────────────────

	#[allow(
		clippy::too_many_lines,
		reason = "table rendering requires extensive layout logic that is clearer kept together"
	)]
	fn render_table(
		&self,
		events: &[Event<'_>],
		start: usize,
		_alignments: &[Alignment],
		available_width: usize,
		lines: &mut Vec<String>,
	) -> usize {
		// Collect table data: header cells and row cells
		let mut header_cells: Vec<String> = Vec::new();
		let mut rows: Vec<Vec<String>> = Vec::new();
		let mut current_row: Vec<String> = Vec::new();
		let mut in_header = false;
		let mut in_cell = false;
		let mut cell_text = String::new();

		let mut i = start + 1;
		while i < events.len() {
			match &events[i] {
				Event::End(TagEnd::Table) => {
					i += 1;
					break;
				},
				Event::Start(Tag::TableHead) => {
					in_header = true;
					i += 1;
				},
				Event::End(TagEnd::TableHead) => {
					in_header = false;
					header_cells.clone_from(&current_row);
					current_row.clear();
					i += 1;
				},
				Event::Start(Tag::TableRow) => {
					current_row.clear();
					i += 1;
				},
				Event::End(TagEnd::TableRow) => {
					if !in_header {
						rows.push(current_row.clone());
					}
					current_row.clear();
					i += 1;
				},
				Event::Start(Tag::TableCell) => {
					in_cell = true;
					cell_text.clear();
					i += 1;
				},
				Event::End(TagEnd::TableCell) => {
					in_cell = false;
					current_row.push(cell_text.clone());
					cell_text.clear();
					i += 1;
				},
				Event::Text(text) if in_cell => {
					cell_text.push_str(text);
					i += 1;
				},
				Event::Code(code) if in_cell => {
					cell_text.push_str(&(self.theme.code)(code));
					i += 1;
				},
				Event::Start(Tag::Strong) if in_cell => {
					// Collect bold content
					let mut bold_text = String::new();
					i += 1;
					while i < events.len() {
						match &events[i] {
							Event::End(TagEnd::Strong) => {
								i += 1;
								break;
							},
							Event::Text(t) => {
								bold_text.push_str(t);
								i += 1;
							},
							_ => {
								i += 1;
							},
						}
					}
					cell_text.push_str(&(self.theme.bold)(&bold_text));
				},
				Event::Start(Tag::Emphasis) if in_cell => {
					let mut em_text = String::new();
					i += 1;
					while i < events.len() {
						match &events[i] {
							Event::End(TagEnd::Emphasis) => {
								i += 1;
								break;
							},
							Event::Text(t) => {
								em_text.push_str(t);
								i += 1;
							},
							_ => {
								i += 1;
							},
						}
					}
					cell_text.push_str(&(self.theme.italic)(&em_text));
				},
				_ => {
					i += 1;
				},
			}
		}
		let consumed = i - start;

		let num_cols = header_cells.len();
		if num_cols == 0 {
			return consumed;
		}

		// Border overhead: "│ " + (n-1) * " │ " + " │" = 3n + 1
		let border_overhead = 3 * num_cols + 1;
		let available_for_cells = available_width.saturating_sub(border_overhead);

		if available_for_cells < num_cols {
			// Too narrow — fallback
			lines.push(String::new());
			return consumed;
		}

		let max_unbroken_word_width = 30;

		// Calculate natural and minimum column widths
		let mut natural_widths = vec![0usize; num_cols];
		let mut min_word_widths = vec![1usize; num_cols];

		for (col, cell) in header_cells.iter().enumerate() {
			natural_widths[col] = visible_width_str(cell);
			min_word_widths[col] = max(1, longest_word_width(cell, max_unbroken_word_width));
		}
		for row in &rows {
			for (col, cell) in row.iter().enumerate() {
				if col < num_cols {
					natural_widths[col] = max(natural_widths[col], visible_width_str(cell));
					min_word_widths[col] =
						max(min_word_widths[col], longest_word_width(cell, max_unbroken_word_width));
				}
			}
		}

		let mut min_col_widths = min_word_widths.clone();
		let mut min_cells_width: usize = min_col_widths.iter().sum();

		if min_cells_width > available_for_cells {
			// Can't fit even minimum word widths — redistribute
			min_col_widths = vec![1; num_cols];
			let remaining = available_for_cells.saturating_sub(num_cols);

			if remaining > 0 {
				let total_weight: usize = min_word_widths.iter().map(|w| w.saturating_sub(1)).sum();
				let growth: Vec<usize> = min_word_widths
					.iter()
					.map(|w| {
						let weight = w.saturating_sub(1);
						(weight * remaining).checked_div(total_weight).unwrap_or(0)
					})
					.collect();

				for (col, g) in growth.iter().enumerate() {
					min_col_widths[col] += g;
				}

				let allocated: usize = growth.iter().sum();
				let mut leftover = remaining.saturating_sub(allocated);
				for col_w in &mut min_col_widths {
					if leftover == 0 {
						break;
					}
					*col_w += 1;
					leftover -= 1;
				}
			}

			min_cells_width = min_col_widths.iter().sum();
		}

		// Calculate final column widths
		let total_natural: usize = natural_widths.iter().sum::<usize>() + border_overhead;
		let column_widths: Vec<usize> = if total_natural <= available_width {
			// Everything fits naturally
			natural_widths
				.iter()
				.enumerate()
				.map(|(i, &w)| max(w, min_col_widths[i]))
				.collect()
		} else {
			// Need to shrink
			let total_grow_potential: usize = natural_widths
				.iter()
				.enumerate()
				.map(|(i, &w)| w.saturating_sub(min_col_widths[i]))
				.sum();
			let extra_width = available_for_cells.saturating_sub(min_cells_width);

			let mut widths: Vec<usize> = min_col_widths
				.iter()
				.enumerate()
				.map(|(i, &min_w)| {
					let delta = natural_widths[i].saturating_sub(min_w);
					let grow = (delta * extra_width)
						.checked_div(total_grow_potential)
						.unwrap_or(0);
					min_w + grow
				})
				.collect();

			// Distribute rounding leftovers
			let allocated: usize = widths.iter().sum();
			let mut remaining = available_for_cells.saturating_sub(allocated);
			loop {
				if remaining == 0 {
					break;
				}
				let mut grew = false;
				for col in 0..num_cols {
					if remaining == 0 {
						break;
					}
					if widths[col] < natural_widths[col] {
						widths[col] += 1;
						remaining -= 1;
						grew = true;
					}
				}
				if !grew {
					break;
				}
			}

			widths
		};

		let t = &self.theme.symbols.table;
		let h = t.horizontal;
		let v = t.vertical;

		// Top border
		let top_cells: Vec<String> = column_widths.iter().map(|w| h.repeat(*w)).collect();
		lines.push(format!(
			"{}{h}{}{h}{}",
			t.top_left,
			top_cells.join(&format!("{h}{}{h}", t.tee_down)),
			t.top_right
		));

		// Header rows (wrapped)
		render_table_row(&header_cells, &column_widths, v, true, &*self.theme.bold, lines);

		// Separator
		let sep_cells: Vec<String> = column_widths.iter().map(|w| h.repeat(*w)).collect();
		let separator = format!(
			"{}{h}{}{h}{}",
			t.tee_right,
			sep_cells.join(&format!("{h}{}{h}", t.cross)),
			t.tee_left
		);
		lines.push(separator.clone());

		// Data rows
		for (row_idx, row) in rows.iter().enumerate() {
			render_table_row(row, &column_widths, v, false, &*self.theme.bold, lines);
			if row_idx < rows.len() - 1 {
				lines.push(separator.clone());
			}
		}

		// Bottom border
		let bottom_cells: Vec<String> = column_widths.iter().map(|w| h.repeat(*w)).collect();
		lines.push(format!(
			"{}{h}{}{h}{}",
			t.bottom_left,
			bottom_cells.join(&format!("{h}{}{h}", t.tee_up)),
			t.bottom_right
		));

		lines.push(String::new()); // spacing after table

		consumed
	}

	// ── Mermaid image ───────────────────────────────────────────────

	fn render_mermaid_image(
		&self,
		image: &MermaidImage,
		available_width: usize,
	) -> Option<Vec<String>> {
		let proto = self.terminal_info.as_ref()?.image_protocol?;

		let scale = 0.5_f64;
		let natural_columns =
			((f64::from(image.width_px) * scale) / f64::from(self.cell_dims.width_px)).ceil() as usize;
		let natural_rows = ((f64::from(image.height_px) * scale)
			/ f64::from(self.cell_dims.height_px))
		.ceil() as usize;

		let columns = min(natural_columns, available_width);
		let rows = if columns < natural_columns {
			let s = columns as f64 / natural_columns as f64;
			max(1, (natural_rows as f64 * s).ceil() as usize)
		} else {
			natural_rows
		};

		let sequence = match proto {
			ImageProtocol::Kitty => {
				encode_kitty(&image.base64, Some(columns as u32), Some(rows as u32), None)
			},
			ImageProtocol::Iterm2 => {
				let w = columns.to_string();
				encode_iterm2(&image.base64, Some(&w), Some("auto"), None, true, true)
			},
		};

		let mut lines = Vec::new();
		for _ in 0..rows.saturating_sub(1) {
			lines.push(String::new());
		}
		let move_up = if rows > 1 {
			format!("\x1b[{}A", rows - 1)
		} else {
			String::new()
		};
		lines.push(format!("{move_up}{sequence}"));

		Some(lines)
	}

	// ── Inline text collection ──────────────────────────────────────

	/// Collect inline text from events starting at `start` (which should be a
	/// Start tag). Returns (rendered text, number of events consumed).
	fn collect_inline_text(&mut self, events: &[Event<'_>], start: usize) -> (String, usize) {
		self.collect_inline_text_with_style(events, start, false)
	}

	fn collect_inline_text_with_style(
		&mut self,
		events: &[Event<'_>],
		start: usize,
		quote_style: bool,
	) -> (String, usize) {
		let mut result = String::new();
		let mut i = start + 1; // skip Start tag
		let default_prefix = self.get_default_style_prefix();

		while i < events.len() {
			match &events[i] {
				Event::End(_) => {
					i += 1;
					break;
				},
				Event::Text(text) => {
					if quote_style {
						// Apply quote+italic style per segment
						let styled = self.apply_quote_style(text);
						result.push_str(&styled);
					} else {
						result.push_str(&self.apply_text_with_newlines_default(text));
					}
					i += 1;
				},
				Event::Code(code) => {
					result.push_str(&(self.theme.code)(code));
					if quote_style {
						result.push_str(&self.get_quote_prefix());
					} else {
						result.push_str(&default_prefix);
					}
					i += 1;
				},
				Event::Start(Tag::Strong) => {
					let (inner, consumed) =
						self.collect_inner_inline(events, i, TagEnd::Strong, quote_style);
					result.push_str(&(self.theme.bold)(&inner));
					if quote_style {
						result.push_str(&self.get_quote_prefix());
					} else {
						result.push_str(&default_prefix);
					}
					i += consumed;
				},
				Event::Start(Tag::Emphasis) => {
					let (inner, consumed) =
						self.collect_inner_inline(events, i, TagEnd::Emphasis, quote_style);
					result.push_str(&(self.theme.italic)(&inner));
					if quote_style {
						result.push_str(&self.get_quote_prefix());
					} else {
						result.push_str(&default_prefix);
					}
					i += consumed;
				},
				Event::Start(Tag::Strikethrough) => {
					let (inner, consumed) =
						self.collect_inner_inline(events, i, TagEnd::Strikethrough, quote_style);
					result.push_str(&(self.theme.strikethrough)(&inner));
					if quote_style {
						result.push_str(&self.get_quote_prefix());
					} else {
						result.push_str(&default_prefix);
					}
					i += consumed;
				},
				Event::Start(Tag::Link { dest_url, .. }) => {
					let href = dest_url.to_string();
					let (link_text, consumed) =
						self.collect_inner_inline(events, i, TagEnd::Link, quote_style);

					// Compare raw text to href (strip mailto: for autolinked emails)
					let href_cmp = href.strip_prefix("mailto:").unwrap_or(&href);
					let raw_text = strip_ansi(&link_text);

					result.push_str(&(self.theme.link)(&(self.theme.underline)(&link_text)));
					if raw_text != href && raw_text != href_cmp {
						result.push_str(&(self.theme.link_url)(&format!(" ({href})")));
					}
					if quote_style {
						result.push_str(&self.get_quote_prefix());
					} else {
						result.push_str(&default_prefix);
					}
					i += consumed;
				},
				Event::SoftBreak => {
					result.push(' ');
					i += 1;
				},
				Event::HardBreak => {
					result.push('\n');
					i += 1;
				},
				Event::Html(html) | Event::InlineHtml(html) => {
					if quote_style {
						let styled = self.apply_quote_style(html);
						result.push_str(&styled);
					} else {
						result.push_str(&self.apply_text_with_newlines_default(html));
					}
					i += 1;
				},
				_ => {
					i += 1;
				},
			}
		}

		(result, i - start)
	}

	/// Apply quote style (italic + quote theme) to text, handling newlines.
	fn apply_quote_style(&self, text: &str) -> String {
		text
			.split('\n')
			.map(|seg| (self.theme.quote)(&(self.theme.italic)(seg)))
			.collect::<Vec<_>>()
			.join("\n")
	}

	/// Get the style prefix for blockquote style (quote + italic).
	fn get_quote_prefix(&self) -> String {
		let sentinel = "\u{0000}";
		let styled = (self.theme.quote)(&(self.theme.italic)(sentinel));
		if let Some(idx) = styled.find(sentinel) {
			styled[..idx].to_owned()
		} else {
			String::new()
		}
	}

	/// Collect inner inline content until we hit the matching end tag.
	fn collect_inner_inline(
		&self,
		events: &[Event<'_>],
		start: usize,
		end_tag: TagEnd,
		quote_style: bool,
	) -> (String, usize) {
		let mut inner = String::new();
		let mut i = start + 1;

		while i < events.len() {
			if events[i] == Event::End(end_tag) {
				i += 1;
				break;
			}
			match &events[i] {
				Event::Text(text) => {
					if quote_style {
						inner.push_str(text);
					} else {
						inner.push_str(&self.apply_default_style(text));
					}
					i += 1;
				},
				Event::Code(code) => {
					inner.push_str(&(self.theme.code)(code));
					i += 1;
				},
				Event::SoftBreak => {
					inner.push(' ');
					i += 1;
				},
				Event::HardBreak => {
					inner.push('\n');
					i += 1;
				},
				_ => {
					i += 1;
				},
			}
		}

		(inner, i - start)
	}

	fn apply_text_with_newlines_default(&self, text: &str) -> String {
		text
			.split('\n')
			.map(|seg| self.apply_default_style(seg))
			.collect::<Vec<_>>()
			.join("\n")
	}
}

impl Component for Markdown {
	fn render(&mut self, width: u16) -> Vec<String> {
		if let Some(ref cached) = self.cached_lines
			&& self.cached_text.as_deref() == Some(&self.text)
			&& self.cached_width == Some(width)
		{
			return cached.clone();
		}

		let result = self.render_impl(width);
		self.cached_text = Some(self.text.clone());
		self.cached_width = Some(width);
		self.cached_lines = Some(result.clone());
		result
	}
}

// ============================================================================
// Helper functions
// ============================================================================

const fn heading_level_depth(level: HeadingLevel) -> usize {
	match level {
		HeadingLevel::H1 => 1,
		HeadingLevel::H2 => 2,
		HeadingLevel::H3 => 3,
		HeadingLevel::H4 => 4,
		HeadingLevel::H5 => 5,
		HeadingLevel::H6 => 6,
	}
}

/// Check if a line looks like a nested list line (starts with spaces + list
/// bullet).
fn is_nested_list_line(line: &str) -> bool {
	let trimmed = line.trim_start();
	trimmed.starts_with("- ")
		|| trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) && trimmed.contains(". ")
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi(text: &str) -> String {
	let mut result = String::with_capacity(text.len());
	let mut in_escape = false;

	for ch in text.chars() {
		if in_escape {
			if ch.is_ascii_alphabetic() || ch == 'm' {
				in_escape = false;
			}
		} else if ch == '\x1b' {
			in_escape = true;
		} else {
			result.push(ch);
		}
	}

	result
}

/// Apply background color to a line, padding to full width.
fn apply_background_to_line(line: &str, width: usize, bg_fn: &dyn Fn(&str) -> String) -> String {
	let vis_len = visible_width_str(line);
	let padding = make_padding(width.saturating_sub(vis_len));
	let with_padding = format!("{line}{padding}");
	bg_fn(&with_padding)
}

/// Get the visible width of the longest word in a string.
fn longest_word_width(text: &str, max_width: usize) -> usize {
	let mut longest = 0usize;
	for word in text.split_whitespace() {
		longest = max(longest, visible_width_str(word));
	}
	min(longest, max_width)
}

/// Simple hash function for mermaid diagram source (mimics Bun.hash).
fn simple_hash(text: &str) -> String {
	let mut hash: u64 = 5381;
	for byte in text.as_bytes() {
		hash = hash.wrapping_mul(33).wrapping_add(u64::from(*byte));
	}
	format!("{hash:x}")
}

/// Render a table row (possibly wrapping cells to multiple lines).
fn render_table_row(
	cells: &[String],
	column_widths: &[usize],
	vertical: &str,
	bold_cells: bool,
	bold_fn: &dyn Fn(&str) -> String,
	lines: &mut Vec<String>,
) {
	let num_cols = column_widths.len();

	// Wrap each cell
	let cell_lines: Vec<Vec<String>> = cells
		.iter()
		.enumerate()
		.map(|(col, text)| {
			let w = if col < num_cols {
				column_widths[col]
			} else {
				10
			};
			let wrapped = rho_text::wrap::wrap_text_with_ansi_str(text, max(1, w));
			wrapped.into_vec()
		})
		.collect();

	let max_lines = cell_lines.iter().map(Vec::len).max().unwrap_or(1);

	for line_idx in 0..max_lines {
		let mut parts = Vec::new();
		for (col, cl) in cell_lines.iter().enumerate() {
			let text = cl.get(line_idx).map_or("", String::as_str);
			let col_w = if col < num_cols {
				column_widths[col]
			} else {
				10
			};
			let vis_w = visible_width_str(text);
			let padded = format!("{text}{}", make_padding(col_w.saturating_sub(vis_w)));
			if bold_cells {
				parts.push(bold_fn(&padded));
			} else {
				parts.push(padded);
			}
		}
		lines.push(format!("{vertical} {} {vertical}", parts.join(&format!(" {vertical} "))));
	}
}

#[cfg(test)]
fn plain_theme_for_rerender() -> MarkdownTheme {
	MarkdownTheme {
		heading:           Rc::new(|s: &str| s.to_owned()),
		link:              Rc::new(|s: &str| s.to_owned()),
		link_url:          Rc::new(|s: &str| s.to_owned()),
		code:              Rc::new(|s: &str| s.to_owned()),
		code_block:        Rc::new(|s: &str| s.to_owned()),
		code_block_border: Rc::new(|s: &str| s.to_owned()),
		quote:             Rc::new(|s: &str| s.to_owned()),
		quote_border:      Rc::new(|s: &str| s.to_owned()),
		hr:                Rc::new(|s: &str| s.to_owned()),
		list_bullet:       Rc::new(|s: &str| s.to_owned()),
		bold:              Rc::new(|s: &str| s.to_owned()),
		italic:            Rc::new(|s: &str| s.to_owned()),
		strikethrough:     Rc::new(|s: &str| s.to_owned()),
		underline:         Rc::new(|s: &str| s.to_owned()),
		highlight_code:    None,
		get_mermaid_image: None,
		symbols:           default_symbols(),
		highlight_colors:  None,
	}
}

#[cfg(test)]
const fn default_symbols() -> SymbolTheme {
	SymbolTheme {
		cursor:         ">",
		input_cursor:   "|",
		box_round:      crate::symbols::RoundedBoxSymbols {
			top_left:     "╭",
			top_right:    "╮",
			bottom_left:  "╰",
			bottom_right: "╯",
			horizontal:   "─",
			vertical:     "│",
		},
		box_sharp:      crate::symbols::BoxSymbols {
			top_left:     "┌",
			top_right:    "┐",
			bottom_left:  "└",
			bottom_right: "┘",
			horizontal:   "─",
			vertical:     "│",
			tee_down:     "┬",
			tee_up:       "┴",
			tee_left:     "┤",
			tee_right:    "├",
			cross:        "┼",
		},
		table:          crate::symbols::BoxSymbols {
			top_left:     "┌",
			top_right:    "┐",
			bottom_left:  "└",
			bottom_right: "┘",
			horizontal:   "─",
			vertical:     "│",
			tee_down:     "┬",
			tee_up:       "┴",
			tee_left:     "┤",
			tee_right:    "├",
			cross:        "┼",
		},
		tree:           crate::symbols::TreeSymbols {
			branch: "├─", last: "╰─", vertical: "│"
		},
		quote_border:   "│",
		hr_char:        "─",
		spinner_frames: &["⠋"],
	}
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
	use super::*;

	fn plain_theme() -> MarkdownTheme {
		plain_theme_for_rerender()
	}

	fn md(text: &str) -> Markdown {
		Markdown::new(text, 0, 0, plain_theme(), None, 2)
	}

	fn render(text: &str, width: u16) -> Vec<String> {
		let mut m = md(text);
		m.render(width)
	}

	fn render_joined(text: &str, width: u16) -> String {
		render(text, width).join("\n")
	}

	// ── Headings ────────────────────────────────────────────────────

	#[test]
	fn test_heading_h1() {
		let lines = render("# Hello", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("Hello"), "H1 should contain text");
	}

	#[test]
	fn test_heading_h2() {
		let lines = render("## World", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("World"), "H2 should contain text");
	}

	#[test]
	fn test_heading_h3() {
		let lines = render("### Sub", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("### Sub"), "H3+ should have prefix");
	}

	#[test]
	fn test_heading_spacing() {
		let lines = render("# Title\n\nParagraph", 60);
		// Should have content for both
		let joined = lines.join("\n");
		assert!(joined.contains("Title"));
		assert!(joined.contains("Paragraph"));
	}

	// ── Paragraphs ──────────────────────────────────────────────────

	#[test]
	fn test_paragraph() {
		let lines = render("Hello world", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("Hello world"));
	}

	#[test]
	fn test_paragraph_spacing() {
		let lines = render("First\n\nSecond", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("First"));
		assert!(joined.contains("Second"));
	}

	// ── Code blocks ─────────────────────────────────────────────────

	#[test]
	fn test_code_block() {
		let lines = render("```rust\nfn main() {}\n```", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("```rust"));
		assert!(joined.contains("fn main()"));
		assert!(joined.contains("```"));
	}

	#[test]
	fn test_code_block_indent() {
		let lines = render("```\nhello\n```", 60);
		// Code should be indented by 2 spaces
		let code_line = lines
			.iter()
			.find(|l| l.contains("hello"))
			.expect("should have code line");
		assert!(code_line.starts_with("  "), "code should be indented");
	}

	#[test]
	fn test_code_block_spacing() {
		let lines = render("```\ncode\n```\n\ntext", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("code"));
		assert!(joined.contains("text"));
	}

	// ── Lists ───────────────────────────────────────────────────────

	#[test]
	fn test_unordered_list() {
		let lines = render("- alpha\n- beta\n- gamma", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("alpha"));
		assert!(joined.contains("beta"));
		assert!(joined.contains("gamma"));
	}

	#[test]
	fn test_ordered_list() {
		let lines = render("1. first\n2. second\n3. third", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("1."));
		assert!(joined.contains("first"));
		assert!(joined.contains("second"));
		assert!(joined.contains("third"));
	}

	#[test]
	fn test_nested_list() {
		let lines = render("- outer\n  - inner\n- outer2", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("outer"));
		assert!(joined.contains("inner"));
		assert!(joined.contains("outer2"));
	}

	// ── Tables ──────────────────────────────────────────────────────

	#[test]
	fn test_simple_table() {
		let lines = render("| A | B |\n|---|---|\n| 1 | 2 |", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("A"));
		assert!(joined.contains("B"));
		assert!(joined.contains("1"));
		assert!(joined.contains("2"));
		// Should have box drawing borders
		assert!(joined.contains("┌"));
		assert!(joined.contains("└"));
	}

	#[test]
	fn test_table_borders() {
		let lines = render("| Name | Value |\n|------|-------|\n| x | 1 |", 60);
		let joined = lines.join("\n");
		// Verify we have proper borders
		assert!(joined.contains("│"));
		assert!(joined.contains("─"));
	}

	#[test]
	fn test_table_wrapping() {
		let text = "| Col |\n|-----|\n| This is a very long cell that needs wrapping |";
		let lines = render(text, 20);
		// Table should render even in narrow width
		assert!(!lines.is_empty());
	}

	// ── Blockquotes ─────────────────────────────────────────────────

	#[test]
	fn test_blockquote() {
		let lines = render("> quoted text", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("quoted text"));
		assert!(joined.contains("│"), "should have quote border");
	}

	#[test]
	fn test_blockquote_multiline() {
		let lines = render("> line one\n> line two", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("line one"));
		assert!(joined.contains("line two"));
	}

	// ── Horizontal rules ────────────────────────────────────────────

	#[test]
	fn test_hr() {
		let lines = render("---", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("─"), "should have hr character");
	}

	#[test]
	fn test_hr_capped_at_80() {
		let lines = render("---", 200);
		// HR should be capped at 80 chars
		let hr_line = lines.iter().find(|l| l.contains('─')).unwrap();
		let hr_count = hr_line.chars().filter(|&c| c == '─').count();
		assert!(hr_count <= 80);
	}

	// ── Inline styles ───────────────────────────────────────────────

	#[test]
	fn test_inline_code() {
		let lines = render("use `foo` here", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("foo"));
	}

	#[test]
	fn test_bold_text() {
		let lines = render("**bold**", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("bold"));
	}

	#[test]
	fn test_italic_text() {
		let lines = render("*italic*", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("italic"));
	}

	// ── Links ───────────────────────────────────────────────────────

	#[test]
	fn test_link_same_text_and_url() {
		let lines = render("https://example.com", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("example.com"));
	}

	#[test]
	fn test_link_different_text() {
		let lines = render("[click here](https://example.com)", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("click here"));
		assert!(joined.contains("https://example.com"));
	}

	// ── Padding ─────────────────────────────────────────────────────

	#[test]
	fn test_horizontal_padding() {
		let mut m = Markdown::new("hello", 2, 0, plain_theme(), None, 2);
		let lines = m.render(20);
		assert!(!lines.is_empty());
		let first_content = lines.iter().find(|l| l.contains("hello")).unwrap();
		assert!(first_content.starts_with("  "), "should have left padding");
	}

	#[test]
	fn test_vertical_padding() {
		let mut m = Markdown::new("hello", 0, 1, plain_theme(), None, 2);
		let lines = m.render(20);
		assert!(lines.len() >= 3, "should have top padding, content, bottom padding");
	}

	// ── Caching ─────────────────────────────────────────────────────

	#[test]
	fn test_cache_hit() {
		let mut m = md("hello");
		let lines1 = m.render(60);
		let lines2 = m.render(60);
		assert_eq!(lines1, lines2);
	}

	#[test]
	fn test_cache_invalidated_by_set_text() {
		let mut m = md("hello");
		let _ = m.render(60);
		m.set_text("world");
		let lines = m.render(60);
		let joined = lines.join("\n");
		assert!(joined.contains("world"));
		assert!(!joined.contains("hello"));
	}

	#[test]
	fn test_cache_invalidated_by_width() {
		let mut m = md("hello world foo bar");
		let lines1 = m.render(60);
		let lines2 = m.render(10);
		// Different width should produce different wrapping
		assert_ne!(lines1.len(), lines2.len());
	}

	// ── Empty text ──────────────────────────────────────────────────

	#[test]
	fn test_empty_text() {
		let lines = render("", 60);
		assert!(lines.is_empty());
	}

	#[test]
	fn test_whitespace_only() {
		let lines = render("   \n  \n  ", 60);
		assert!(lines.is_empty());
	}

	// ── Combined ────────────────────────────────────────────────────

	#[test]
	fn test_heading_list_combo() {
		let text = "# Title\n\n- item 1\n- item 2";
		let lines = render(text, 60);
		let joined = lines.join("\n");
		assert!(joined.contains("Title"));
		assert!(joined.contains("item 1"));
		assert!(joined.contains("item 2"));
	}

	#[test]
	fn test_strip_ansi() {
		assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
		assert_eq!(strip_ansi("plain"), "plain");
	}

	#[test]
	fn test_simple_hash() {
		let h1 = simple_hash("hello");
		let h2 = simple_hash("hello");
		let h3 = simple_hash("world");
		assert_eq!(h1, h2);
		assert_ne!(h1, h3);
	}

	#[test]
	fn test_longest_word_width() {
		assert_eq!(longest_word_width("hello world", 30), 5);
		assert_eq!(longest_word_width("superlongword", 10), 10);
		assert_eq!(longest_word_width("", 30), 0);
	}

	// ── HTML in text ────────────────────────────────────────────────

	#[test]
	fn test_html_in_text() {
		let lines = render("<thinking>content</thinking>", 60);
		let joined = lines.join("\n");
		assert!(joined.contains("thinking") || joined.contains("content"));
	}
}
