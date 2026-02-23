//! Bordered output block with header, sections, and state-based styling.
//!
//! Used by tool renderers to display tool call results with appropriate
//! visual treatment (pending/running/success/error).

use crate::symbols::BoxSymbols;

/// Visual state of an output block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputBlockState {
	Pending,
	Running,
	Success,
	Error,
	Warning,
}

/// A section within an output block.
pub struct OutputSection {
	/// Optional styled label for the section divider.
	pub label: Option<String>,
	/// Content lines (already styled).
	pub lines: Vec<String>,
}

/// Rendering options for an output block.
pub struct OutputBlockOptions {
	/// Styled header text (e.g., tool name with icon).
	pub header:       String,
	/// Visible width of the header.
	pub header_width: usize,
	/// Current state (determines border/bg color).
	pub state:        OutputBlockState,
	/// Content sections.
	pub sections:     Vec<OutputSection>,
	/// Border styling function.
	pub border_style: Box<dyn Fn(&str) -> String>,
	/// Background styling function (applied to content lines).
	#[allow(clippy::type_complexity, reason = "closure type alias would reduce readability here")]
	pub bg_style:     Option<Box<dyn Fn(&str) -> String>>,
}

/// Render an output block to lines.
///
/// Returns `Vec<String>` of rendered lines including borders.
pub fn render_output_block(opts: &OutputBlockOptions, width: u16) -> Vec<String> {
	let w = width as usize;
	if w < 4 {
		return Vec::new();
	}

	let box_chars = BoxSymbols {
		top_left:     "\u{256d}",
		top_right:    "\u{256e}",
		bottom_left:  "\u{2570}",
		bottom_right: "\u{256f}",
		horizontal:   "\u{2500}",
		vertical:     "\u{2502}",
		tee_down:     "\u{252c}",
		tee_up:       "\u{2534}",
		tee_left:     "\u{2524}",
		tee_right:    "\u{251c}",
		cross:        "\u{253c}",
	};

	let border = &opts.border_style;
	let h = box_chars.horizontal;
	let v = box_chars.vertical;
	let content_width = w.saturating_sub(4); // │ + space + content + space + │

	let mut lines = Vec::new();

	// Top border: ╭─── header ───────────╮
	let header_fill = w.saturating_sub(4 + opts.header_width); // 4 = ╭─ + space + ─╮
	let top = format!(
		"{}{} {} {}{}",
		border(box_chars.top_left),
		border(h),
		opts.header,
		border(&h.repeat(header_fill)),
		border(box_chars.top_right),
	);
	lines.push(top);

	// Sections
	for (i, section) in opts.sections.iter().enumerate() {
		// Section divider (skip for first section if no label)
		if i > 0 || section.label.is_some() {
			if let Some(ref label) = section.label {
				let label_width = rho_text::width::visible_width_str(label);
				let fill = w.saturating_sub(4 + label_width);
				let divider = format!(
					"{}{} {} {}{}",
					border(box_chars.tee_right),
					border(h),
					label,
					border(&h.repeat(fill)),
					border(box_chars.tee_left),
				);
				lines.push(divider);
			} else if i > 0 {
				// Plain divider between sections
				let divider = format!(
					"{}{}{}",
					border(box_chars.tee_right),
					border(&h.repeat(w.saturating_sub(2))),
					border(box_chars.tee_left),
				);
				lines.push(divider);
			}
		}

		// Content lines
		for line in &section.lines {
			let visible_w = rho_text::width::visible_width_str(line);
			let pad = content_width.saturating_sub(visible_w);
			let content_line = format!("{} {}{} {}", border(v), line, " ".repeat(pad), border(v),);
			if let Some(ref bg) = opts.bg_style {
				lines.push(bg(&content_line));
			} else {
				lines.push(content_line);
			}
		}
	}

	// Bottom border: ╰───────────────────╯
	let bottom = format!(
		"{}{}{}",
		border(box_chars.bottom_left),
		border(&h.repeat(w.saturating_sub(2))),
		border(box_chars.bottom_right),
	);
	lines.push(bottom);

	lines
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_output_block_basic() {
		let opts = OutputBlockOptions {
			header:       "Bash".to_owned(),
			header_width: 4,
			state:        OutputBlockState::Success,
			sections:     vec![OutputSection { label: None, lines: vec!["hello world".to_owned()] }],
			border_style: Box::new(|s| s.to_owned()),
			bg_style:     None,
		};
		let result = render_output_block(&opts, 40);
		assert!(!result.is_empty());
		assert!(result[0].contains("Bash"));
		assert!(result.last().unwrap().contains("\u{2570}"));
	}

	#[test]
	fn test_output_block_with_sections() {
		let opts = OutputBlockOptions {
			header:       "Edit".to_owned(),
			header_width: 4,
			state:        OutputBlockState::Success,
			sections:     vec![
				OutputSection { label: Some("Command".to_owned()), lines: vec!["ls -la".to_owned()] },
				OutputSection {
					label: Some("Output".to_owned()),
					lines: vec!["file1.txt".to_owned(), "file2.txt".to_owned()],
				},
			],
			border_style: Box::new(|s| s.to_owned()),
			bg_style:     None,
		};
		let result = render_output_block(&opts, 40);
		// Should have: top + divider + 1 line + divider + 2 lines + bottom = 7
		assert!(result.len() >= 7);
	}

	#[test]
	fn test_narrow_width() {
		let opts = OutputBlockOptions {
			header:       "X".to_owned(),
			header_width: 1,
			state:        OutputBlockState::Pending,
			sections:     vec![],
			border_style: Box::new(|s| s.to_owned()),
			bg_style:     None,
		};
		let result = render_output_block(&opts, 3);
		assert!(result.is_empty()); // Too narrow
	}
}
