//! TUI — Main class for managing terminal UI with differential rendering.
//!
//! Port of `tui.ts` (1,188 lines). Implements:
//! - Differential rendering (only changed lines are redrawn)
//! - Overlay compositing (modal components rendered on top)
//! - Hardware cursor positioning (for IME)
//! - Synchronized output (CSI 2026)

use std::{
	cmp::{max, min},
	fmt::Write as _,
};

use crate::{
	capabilities::TerminalInfo,
	component::{CURSOR_MARKER, Component},
	overlay::{OverlayOptions, resolve_overlay_layout},
	terminal::Terminal,
};

/// ANSI reset + hyperlink reset.
const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

/// Result from an input listener.
pub struct InputListenerResult {
	/// If true, consume the input (don't forward to focused component).
	pub consume: bool,
	/// Optionally replace the input data.
	pub data:    Option<String>,
}

/// Input listener callback type.
pub type InputListener = Box<dyn FnMut(&str) -> Option<InputListenerResult>>;

/// Entry in the overlay stack.
struct OverlayEntry {
	component: Box<dyn Component>,
	options:   OverlayOptions,
	hidden:    bool,
}

/// Handle for controlling a shown overlay.
pub struct OverlayHandle {
	/// Index into the overlay stack.
	index: usize,
}

impl OverlayHandle {
	/// Get the overlay index (used by TUI to look up the entry).
	pub const fn index(&self) -> usize {
		self.index
	}
}

/// Main TUI manager with differential rendering.
///
/// Tui is a pure differential renderer. The caller (App) owns components
/// directly, renders them to lines, and passes those lines to
/// [`Tui::render_lines`]. Tui handles overlay compositing, cursor
/// extraction, and differential terminal writes.
pub struct Tui {
	// Rendering state
	previous_lines:        Vec<String>,
	previous_width:        u16,
	cursor_row:            usize,
	hardware_cursor_row:   usize,
	max_lines_rendered:    usize,
	previous_viewport_top: usize,
	full_redraw_count:     u32,
	render_needed:         bool,
	stopped:               bool,

	// Options
	show_hardware_cursor: bool,
	clear_on_shrink:      bool,

	// Overlays
	overlay_stack: Vec<OverlayEntry>,

	// Input
	input_listeners: Vec<InputListener>,

	// Terminal info (for image line detection)
	terminal_info: TerminalInfo,
}

impl Tui {
	pub fn new(terminal_info: TerminalInfo) -> Self {
		Self {
			previous_lines: Vec::new(),
			previous_width: 0,
			cursor_row: 0,
			hardware_cursor_row: 0,
			max_lines_rendered: 0,
			previous_viewport_top: 0,
			full_redraw_count: 0,
			render_needed: false,
			stopped: false,
			show_hardware_cursor: false,
			clear_on_shrink: false,
			overlay_stack: Vec::new(),
			input_listeners: Vec::new(),
			terminal_info,
		}
	}

	// ── Overlay management ──────────────────────────────────────────

	/// Show an overlay component. Returns a handle to control it.
	pub fn show_overlay(
		&mut self,
		component: Box<dyn Component>,
		options: OverlayOptions,
	) -> OverlayHandle {
		let index = self.overlay_stack.len();
		self
			.overlay_stack
			.push(OverlayEntry { component, options, hidden: false });
		self.render_needed = true;

		OverlayHandle { index }
	}

	/// Hide an overlay by handle.
	pub fn hide_overlay(&mut self, handle: &OverlayHandle) {
		if handle.index < self.overlay_stack.len() {
			self.overlay_stack.remove(handle.index);
			self.render_needed = true;
		}
	}

	/// Hide the topmost overlay.
	pub fn hide_top_overlay(&mut self) {
		if self.overlay_stack.pop().is_some() {
			self.render_needed = true;
		}
	}

	/// Set overlay hidden state.
	pub fn set_overlay_hidden(&mut self, handle: &OverlayHandle, hidden: bool) {
		if let Some(entry) = self.overlay_stack.get_mut(handle.index)
			&& entry.hidden != hidden
		{
			entry.hidden = hidden;
			self.render_needed = true;
		}
	}

	/// Check if there are any visible overlays.
	pub fn has_overlay(&self) -> bool {
		self.overlay_stack.iter().any(Self::is_overlay_visible)
	}

	const fn is_overlay_visible(entry: &OverlayEntry) -> bool {
		!entry.hidden
	}

	// ── Input handling ──────────────────────────────────────────────

	/// Add an input listener. Listeners are called in order before the focused
	/// component.
	pub fn add_input_listener(&mut self, listener: InputListener) {
		self.input_listeners.push(listener);
	}

	// ── Rendering ───────────────────────────────────────────────────

	pub const fn full_redraws(&self) -> u32 {
		self.full_redraw_count
	}

	pub const fn show_hardware_cursor(&self) -> bool {
		self.show_hardware_cursor
	}

	pub const fn set_show_hardware_cursor(&mut self, enabled: bool) {
		self.show_hardware_cursor = enabled;
		self.render_needed = true;
	}

	pub const fn clear_on_shrink(&self) -> bool {
		self.clear_on_shrink
	}

	pub const fn set_clear_on_shrink(&mut self, enabled: bool) {
		self.clear_on_shrink = enabled;
	}

	pub const fn request_render(&mut self) {
		self.render_needed = true;
	}

	/// Force a full re-render (clears all previous state).
	pub fn request_render_force(&mut self) {
		self.previous_lines.clear();
		self.previous_width = 0; // triggers width-changed path
		self.cursor_row = 0;
		self.hardware_cursor_row = 0;
		self.max_lines_rendered = 0;
		self.previous_viewport_top = 0;
		self.render_needed = true;
	}

	pub fn invalidate(&mut self) {
		for entry in &mut self.overlay_stack {
			entry.component.invalidate();
		}
	}

	/// Check if a render is needed.
	pub const fn needs_render(&self) -> bool {
		self.render_needed
	}

	/// Composite overlays into content lines.
	fn composite_overlays(&mut self, lines: &mut Vec<String>, term_width: u16, term_height: u16) {
		if self.overlay_stack.is_empty() {
			return;
		}

		struct RenderedOverlay {
			overlay_lines: Vec<String>,
			row:           u16,
			col:           u16,
			width:         u16,
		}

		let mut rendered = Vec::new();
		let mut min_lines_needed = lines.len();

		for entry in &mut self.overlay_stack {
			if !Self::is_overlay_visible(entry) {
				continue;
			}

			// Get layout with height=0 to determine width/maxHeight
			let layout0 = resolve_overlay_layout(&entry.options, 0, term_width, term_height);

			// Render overlay at calculated width
			let mut overlay_lines = entry.component.render(layout0.width);

			// Apply maxHeight
			if let Some(mh) = layout0.max_height {
				overlay_lines.truncate(mh as usize);
			}

			// Get final row/col with actual height
			let layout = resolve_overlay_layout(
				&entry.options,
				overlay_lines.len() as u16,
				term_width,
				term_height,
			);

			let end = layout.row as usize + overlay_lines.len();
			min_lines_needed = max(min_lines_needed, end);

			rendered.push(RenderedOverlay {
				overlay_lines,
				row: layout.row,
				col: layout.col,
				width: layout.width,
			});
		}

		// Extend lines with empty entries if needed for overlay placement
		let working_height = max(lines.len(), min_lines_needed);
		lines.resize(working_height, String::new());

		let viewport_start = working_height.saturating_sub(term_height as usize);

		// Composite each overlay
		for r in &rendered {
			for (i, overlay_line) in r.overlay_lines.iter().enumerate() {
				let idx = viewport_start + r.row as usize + i;
				if idx < lines.len() {
					// Truncate overlay line to declared width before compositing
					let trunc = truncate_if_needed(overlay_line, r.width);
					lines[idx] = composite_line_at(
						&lines[idx],
						&trunc,
						r.col,
						r.width,
						term_width,
						&self.terminal_info,
					);
				}
			}
		}

		// Final verification: truncate any composited line exceeding terminal width
		for line in lines.iter_mut() {
			let w = rho_text::width::visible_width_str(line);
			if w > term_width as usize {
				let slice = rho_text::slice::slice_with_width_str(line, 0, term_width as usize, true);
				*line = slice.text;
			}
		}
	}

	/// Apply segment resets to lines (reset ANSI state at end of each line).
	fn apply_line_resets(&self, lines: &mut [String]) {
		for line in lines.iter_mut() {
			if !self.terminal_info.is_image_line(line) {
				line.push_str(SEGMENT_RESET);
			}
		}
	}

	/// Find and extract cursor position from rendered lines.
	fn extract_cursor_position(lines: &mut [String], height: u16) -> Option<(usize, usize)> {
		let viewport_top = lines.len().saturating_sub(height as usize);
		for row in (viewport_top..lines.len()).rev() {
			if let Some(marker_idx) = lines[row].find(CURSOR_MARKER) {
				let before = &lines[row][..marker_idx];
				let col = rho_text::width::visible_width_str(before);

				// Strip marker
				let after_marker = marker_idx + CURSOR_MARKER.len();
				let mut new_line = String::with_capacity(lines[row].len() - CURSOR_MARKER.len());
				new_line.push_str(&lines[row][..marker_idx]);
				new_line.push_str(&lines[row][after_marker..]);
				lines[row] = new_line;

				return Some((row, col));
			}
		}
		None
	}

	/// Perform a render cycle with externally-provided content lines.
	/// Overlays are still composited by Tui.
	pub fn render_lines(
		&mut self,
		content_lines: Vec<String>,
		terminal: &mut dyn Terminal,
	) -> std::io::Result<()> {
		if !self.render_needed || self.stopped {
			return Ok(());
		}
		self.render_needed = false;

		let width = terminal.columns();
		let height = terminal.rows();

		self.do_render_with_lines(content_lines, width, height, terminal)
	}

	/// Run input listeners on raw input data.
	/// Returns `Some(data)` to forward to the focused component, or `None` if
	/// consumed by a listener.
	pub fn process_input_listeners(&mut self, data: &str) -> Option<String> {
		let mut current = data.to_owned();

		for listener in &mut self.input_listeners {
			if let Some(result) = listener(&current) {
				if result.consume {
					return None;
				}
				if let Some(replacement) = result.data {
					current = replacement;
				}
			}
		}

		if current.is_empty() {
			None
		} else {
			Some(current)
		}
	}

	/// Core differential rendering.
	/// Core differential rendering with pre-rendered content lines.
	fn do_render_with_lines(
		&mut self,
		mut new_lines: Vec<String>,
		width: u16,
		height: u16,
		terminal: &mut dyn Terminal,
	) -> std::io::Result<()> {
		let mut viewport_top = self.max_lines_rendered.saturating_sub(height as usize);
		let mut prev_viewport_top = self.previous_viewport_top;
		let mut hardware_cursor_row = self.hardware_cursor_row;

		let compute_line_diff =
			|target_row: usize, hw_cursor: usize, prev_vt: usize, vt: usize| -> i32 {
				let current_screen = hw_cursor as i32 - prev_vt as i32;
				let target_screen = target_row as i32 - vt as i32;
				target_screen - current_screen
			};

		// Composite overlays
		if !self.overlay_stack.is_empty() {
			self.composite_overlays(&mut new_lines, width, height);
		}

		// Extract cursor position before line resets
		let cursor_pos = Self::extract_cursor_position(&mut new_lines, height);

		// Apply resets
		self.apply_line_resets(&mut new_lines);

		let width_changed = self.previous_width != 0 && self.previous_width != width;

		// Helper closure for full render
		let full_render = |this: &mut Self,
		                   terminal: &mut dyn Terminal,
		                   new_lines: &[String],
		                   clear: bool,
		                   cursor_pos: Option<(usize, usize)>|
		 -> std::io::Result<()> {
			this.full_redraw_count += 1;
			let mut buffer = String::from("\x1b[?2026h"); // Begin synchronized output
			if clear {
				buffer.push_str("\x1b[3J\x1b[2J\x1b[H"); // Clear scrollback, screen, home
			}
			for (i, line) in new_lines.iter().enumerate() {
				if i > 0 {
					buffer.push_str("\r\n");
				}
				buffer.push_str(line);
			}
			buffer.push_str("\x1b[?2026l"); // End synchronized output
			terminal.write(&buffer)?;

			this.cursor_row = new_lines.len().saturating_sub(1);
			this.hardware_cursor_row = this.cursor_row;
			if clear {
				this.max_lines_rendered = new_lines.len();
			} else {
				this.max_lines_rendered = max(this.max_lines_rendered, new_lines.len());
			}
			this.previous_viewport_top = this.max_lines_rendered.saturating_sub(height as usize);

			this.position_hardware_cursor(terminal, cursor_pos, new_lines.len())?;
			this.previous_lines = new_lines.to_vec();
			this.previous_width = width;
			Ok(())
		};

		// First render
		if self.previous_lines.is_empty() && !width_changed {
			return full_render(self, terminal, &new_lines, false, cursor_pos);
		}

		// Width changed
		if width_changed {
			return full_render(self, terminal, &new_lines, true, cursor_pos);
		}

		// Content shrunk and no overlays
		if self.clear_on_shrink
			&& new_lines.len() < self.max_lines_rendered
			&& self.overlay_stack.is_empty()
		{
			return full_render(self, terminal, &new_lines, true, cursor_pos);
		}

		// Find first and last changed lines
		let mut first_changed: Option<usize> = None;
		let mut last_changed: usize = 0;
		let max_len = max(new_lines.len(), self.previous_lines.len());
		for i in 0..max_len {
			let old = self.previous_lines.get(i).map_or("", String::as_str);
			let new = new_lines.get(i).map_or("", String::as_str);
			if old != new {
				if first_changed.is_none() {
					first_changed = Some(i);
				}
				last_changed = i;
			}
		}

		let appended = new_lines.len() > self.previous_lines.len();
		if appended {
			if first_changed.is_none() {
				first_changed = Some(self.previous_lines.len());
			}
			last_changed = new_lines.len() - 1;
		}
		let append_start = appended
			&& first_changed == Some(self.previous_lines.len())
			&& first_changed.unwrap_or(0) > 0;

		// No changes
		let Some(first_changed) = first_changed else {
			self.position_hardware_cursor(terminal, cursor_pos, new_lines.len())?;
			self.previous_viewport_top = self.max_lines_rendered.saturating_sub(height as usize);
			return Ok(());
		};

		// All changes are in deleted lines
		if first_changed >= new_lines.len() {
			if self.previous_lines.len() > new_lines.len() {
				let mut buffer = String::from("\x1b[?2026h");
				let target_row = new_lines.len().saturating_sub(1);
				let line_diff =
					compute_line_diff(target_row, hardware_cursor_row, prev_viewport_top, viewport_top);
				if line_diff > 0 {
					let _ = write!(buffer, "\x1b[{line_diff}B");
				} else if line_diff < 0 {
					let _ = write!(buffer, "\x1b[{}A", -line_diff);
				}
				buffer.push('\r');
				let extra = self.previous_lines.len() - new_lines.len();
				if extra > height as usize {
					return full_render(self, terminal, &new_lines, true, cursor_pos);
				}
				if extra > 0 {
					buffer.push_str("\x1b[1B");
				}
				for i in 0..extra {
					buffer.push_str("\r\x1b[2K");
					if i < extra - 1 {
						buffer.push_str("\x1b[1B");
					}
				}
				if extra > 0 {
					let _ = write!(buffer, "\x1b[{extra}A");
				}
				buffer.push_str("\x1b[?2026l");
				terminal.write(&buffer)?;
				self.cursor_row = target_row;
				self.hardware_cursor_row = target_row;
			}
			self.position_hardware_cursor(terminal, cursor_pos, new_lines.len())?;
			self.previous_lines = new_lines;
			self.previous_width = width;
			self.previous_viewport_top = self.max_lines_rendered.saturating_sub(height as usize);
			return Ok(());
		}

		// Check if first change is above previous viewport
		let prev_content_viewport_top = self.previous_lines.len().saturating_sub(height as usize);
		if first_changed < prev_content_viewport_top {
			return full_render(self, terminal, &new_lines, true, cursor_pos);
		}

		// Differential render
		let mut buffer = String::from("\x1b[?2026h");
		let prev_viewport_bottom = prev_viewport_top + height as usize - 1;
		let move_target_row = if append_start {
			first_changed - 1
		} else {
			first_changed
		};

		if move_target_row > prev_viewport_bottom {
			let current_screen_row =
				min(height as usize - 1, hardware_cursor_row.saturating_sub(prev_viewport_top));
			let move_to_bottom = height as usize - 1 - current_screen_row;
			if move_to_bottom > 0 {
				let _ = write!(buffer, "\x1b[{move_to_bottom}B");
			}
			let scroll = move_target_row - prev_viewport_bottom;
			for _ in 0..scroll {
				buffer.push_str("\r\n");
			}
			prev_viewport_top += scroll;
			viewport_top += scroll;
			hardware_cursor_row = move_target_row;
		}

		let line_diff =
			compute_line_diff(move_target_row, hardware_cursor_row, prev_viewport_top, viewport_top);
		if line_diff > 0 {
			let _ = write!(buffer, "\x1b[{line_diff}B");
		} else if line_diff < 0 {
			let _ = write!(buffer, "\x1b[{}A", -line_diff);
		}

		if append_start {
			buffer.push_str("\r\n");
		} else {
			buffer.push('\r');
		}

		// Render changed lines
		let render_end = min(last_changed, new_lines.len() - 1);
		for (idx, line) in new_lines[first_changed..=render_end].iter().enumerate() {
			if idx > 0 {
				buffer.push_str("\r\n");
			}
			buffer.push_str("\x1b[2K"); // Clear current line
			buffer.push_str(line);
		}

		let mut final_cursor_row = render_end;

		// Clear extra lines if content shrunk
		if self.previous_lines.len() > new_lines.len() {
			if render_end < new_lines.len() - 1 {
				let move_down = new_lines.len() - 1 - render_end;
				let _ = write!(buffer, "\x1b[{move_down}B");
				final_cursor_row = new_lines.len() - 1;
			}
			let extra = self.previous_lines.len() - new_lines.len();
			for _ in 0..extra {
				buffer.push_str("\r\n\x1b[2K");
			}
			let _ = write!(buffer, "\x1b[{extra}A");
		}

		buffer.push_str("\x1b[?2026l"); // End synchronized output
		terminal.write(&buffer)?;

		self.cursor_row = new_lines.len().saturating_sub(1);
		self.hardware_cursor_row = final_cursor_row;
		self.max_lines_rendered = max(self.max_lines_rendered, new_lines.len());
		self.previous_viewport_top = self.max_lines_rendered.saturating_sub(height as usize);

		self.position_hardware_cursor(terminal, cursor_pos, new_lines.len())?;
		self.previous_lines = new_lines;
		self.previous_width = width;
		Ok(())
	}

	/// Position the hardware cursor for IME candidate window.
	fn position_hardware_cursor(
		&mut self,
		terminal: &mut dyn Terminal,
		cursor_pos: Option<(usize, usize)>,
		total_lines: usize,
	) -> std::io::Result<()> {
		let Some((row, col)) = cursor_pos else {
			return terminal.hide_cursor();
		};
		if total_lines == 0 {
			return terminal.hide_cursor();
		}

		let target_row = min(row, total_lines - 1);
		let target_col = col;

		let row_delta = target_row as i32 - self.hardware_cursor_row as i32;
		let mut buffer = String::new();
		if row_delta > 0 {
			let _ = write!(buffer, "\x1b[{row_delta}B");
		} else if row_delta < 0 {
			let _ = write!(buffer, "\x1b[{}A", -row_delta);
		}
		let _ = write!(buffer, "\x1b[{}G", target_col + 1);

		if !buffer.is_empty() {
			terminal.write(&buffer)?;
		}

		self.hardware_cursor_row = target_row;
		if self.show_hardware_cursor {
			terminal.show_cursor()
		} else {
			terminal.hide_cursor()
		}
	}

	/// Mark as stopped (prevents further rendering).
	pub fn stop(&mut self, terminal: &mut dyn Terminal) -> std::io::Result<()> {
		self.stopped = true;
		if !self.previous_lines.is_empty() {
			let target_row = self.previous_lines.len();
			let line_diff = target_row as i32 - self.hardware_cursor_row as i32;
			if line_diff > 0 {
				terminal.write(&format!("\x1b[{line_diff}B"))?;
			} else if line_diff < 0 {
				terminal.write(&format!("\x1b[{}A", -line_diff))?;
			}
			terminal.write("\r\n")?;
		}
		terminal.show_cursor()?;
		terminal.stop()
	}
}

/// Splice overlay content into a base line at a specific column.
fn composite_line_at(
	base_line: &str,
	overlay_line: &str,
	start_col: u16,
	overlay_width: u16,
	total_width: u16,
	terminal_info: &TerminalInfo,
) -> String {
	if terminal_info.is_image_line(base_line) {
		return base_line.to_owned();
	}

	let start = start_col as usize;
	let ow = overlay_width as usize;
	let tw = total_width as usize;
	let after_start = start + ow;

	// Extract before and after segments from base line
	let base = rho_text::slice::extract_segments_str(
		base_line,
		start,
		after_start,
		tw.saturating_sub(after_start),
		true,
	);

	// Extract overlay with width tracking
	let overlay = rho_text::slice::slice_with_width_str(overlay_line, 0, ow, true);

	// Pad segments to target widths
	let before_pad = start.saturating_sub(base.before_width);
	let overlay_pad = ow.saturating_sub(overlay.width);
	let actual_before_width = max(start, base.before_width);
	let actual_overlay_width = max(ow, overlay.width);
	let after_target = tw.saturating_sub(actual_before_width + actual_overlay_width);
	let after_pad = after_target.saturating_sub(base.after_width);

	// Compose result
	let mut result = String::with_capacity(
		base.before.len()
			+ before_pad
			+ SEGMENT_RESET.len()
			+ overlay.text.len()
			+ overlay_pad
			+ SEGMENT_RESET.len()
			+ base.after.len()
			+ after_pad,
	);
	result.push_str(&base.before);
	for _ in 0..before_pad {
		result.push(' ');
	}
	result.push_str(SEGMENT_RESET);
	result.push_str(&overlay.text);
	for _ in 0..overlay_pad {
		result.push(' ');
	}
	result.push_str(SEGMENT_RESET);
	result.push_str(&base.after);
	for _ in 0..after_pad {
		result.push(' ');
	}

	// Verify width
	let result_width = rho_text::width::visible_width_str(&result);
	if result_width <= tw {
		return result;
	}
	// Truncate
	rho_text::slice::slice_with_width_str(&result, 0, tw, true).text
}

/// Truncate a line if it exceeds the given width.
fn truncate_if_needed(line: &str, max_width: u16) -> String {
	let w = rho_text::width::visible_width_str(line);
	if w <= max_width as usize {
		line.to_owned()
	} else {
		rho_text::slice::slice_with_width_str(line, 0, max_width as usize, true).text
	}
}

/// Check if input data is a key release event (Kitty protocol).
pub fn is_key_release(data: &str) -> bool {
	// Kitty key release: ESC [ ... :3 <letter>
	// e.g., ESC [ 97 ; 1 : 3 u
	let bytes = data.as_bytes();
	if bytes.len() < 4 || bytes[0] != 0x1b || bytes[1] != b'[' {
		return false;
	}
	// Look for ":3" before the final character
	if let Some(pos) = data.rfind(":3") {
		let after = &data[pos + 2..];
		// Should be exactly one final character (letter or ~)
		after.len() == 1 && (after.as_bytes()[0].is_ascii_alphabetic() || after.as_bytes()[0] == b'~')
	} else {
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_is_key_release() {
		assert!(is_key_release("\x1b[97;1:3u"));
		assert!(is_key_release("\x1b[1;1:3A"));
		assert!(is_key_release("\x1b[3;1:3~"));
		assert!(!is_key_release("\x1b[97u"));
		assert!(!is_key_release("\x1b[A"));
		assert!(!is_key_release("a"));
	}

	#[test]
	fn test_composite_line_at_simple() {
		let terminal_info =
			crate::capabilities::get_terminal_info(crate::capabilities::TerminalId::Base);
		let base = "hello world here";
		let overlay = "OVR";
		let result = composite_line_at(base, overlay, 6, 3, 16, &terminal_info);
		// Should contain the overlay text
		assert!(result.contains("OVR"));
		// Width should not exceed total
		let w = rho_text::width::visible_width_str(&result);
		assert!(w <= 16);
	}

	#[test]
	fn test_extract_cursor_position() {
		let mut lines =
			vec!["line0".to_owned(), format!("before{CURSOR_MARKER}after"), "line2".to_owned()];
		let pos = Tui::extract_cursor_position(&mut lines, 24);
		assert_eq!(pos, Some((1, 6)));
		// Marker should be stripped
		assert_eq!(lines[1], "beforeafter");
	}

	#[test]
	fn test_extract_cursor_position_not_found() {
		let mut lines = vec!["line0".to_owned(), "line1".to_owned()];
		let pos = Tui::extract_cursor_position(&mut lines, 24);
		assert_eq!(pos, None);
	}

	#[test]
	fn test_truncate_if_needed() {
		let line = "hello world";
		assert_eq!(truncate_if_needed(line, 20), "hello world");
		let truncated = truncate_if_needed(line, 5);
		let w = rho_text::width::visible_width_str(&truncated);
		assert!(w <= 5);
	}
}
