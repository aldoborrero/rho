//! Loader component — animated spinner with message.
//!
//! Uses a tick-based animation model. Call `tick()` periodically (e.g., every
//! 50ms in the event loop) to advance the animation. The spinner advances
//! every 80ms.

use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};

use crate::component::{Component, InputResult};

/// Color function type.
pub type ColorFn = Box<dyn Fn(&str) -> String>;

/// Default braille spinner frames.
const DEFAULT_FRAMES: &[&str] = &[
	"\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
	"\u{2807}", "\u{280F}",
];

/// Animation interval.
const FRAME_INTERVAL: Duration = Duration::from_millis(80);

/// Loader component with animated spinner.
pub struct Loader {
	frames:           Vec<String>,
	current_frame:    usize,
	spinner_color_fn: ColorFn,
	message_color_fn: ColorFn,
	message:          String,
	padding_x:        usize,
	running:          bool,
	last_frame_time:  Instant,
}

impl Loader {
	pub fn new(spinner_color_fn: ColorFn, message_color_fn: ColorFn, message: &str) -> Self {
		Self {
			frames: DEFAULT_FRAMES.iter().map(|s| (*s).to_owned()).collect(),
			current_frame: 0,
			spinner_color_fn,
			message_color_fn,
			message: message.to_owned(),
			padding_x: 1,
			running: true,
			last_frame_time: Instant::now(),
		}
	}

	pub fn with_frames(
		spinner_color_fn: ColorFn,
		message_color_fn: ColorFn,
		message: &str,
		frames: Vec<String>,
	) -> Self {
		Self {
			frames,
			current_frame: 0,
			spinner_color_fn,
			message_color_fn,
			message: message.to_owned(),
			padding_x: 1,
			running: true,
			last_frame_time: Instant::now(),
		}
	}

	pub fn set_message(&mut self, message: &str) {
		message.clone_into(&mut self.message);
	}

	pub fn message(&self) -> &str {
		&self.message
	}

	pub fn start(&mut self) {
		self.running = true;
		self.last_frame_time = Instant::now();
	}

	pub const fn stop(&mut self) {
		self.running = false;
	}

	pub const fn is_running(&self) -> bool {
		self.running
	}

	/// Advance animation if enough time has passed. Returns true if frame
	/// changed.
	pub fn tick(&mut self) -> bool {
		if !self.running || self.frames.is_empty() {
			return false;
		}
		let now = Instant::now();
		if now.duration_since(self.last_frame_time) >= FRAME_INTERVAL {
			self.current_frame = (self.current_frame + 1) % self.frames.len();
			self.last_frame_time = now;
			return true;
		}
		false
	}

	fn build_display(&self) -> String {
		if self.frames.is_empty() {
			return (self.message_color_fn)(&self.message);
		}
		let frame = &self.frames[self.current_frame];
		let spinner = (self.spinner_color_fn)(frame);
		let msg = (self.message_color_fn)(&self.message);
		format!("{spinner} {msg}")
	}
}

impl Component for Loader {
	fn render(&mut self, width: u16) -> Vec<String> {
		let w = width as usize;
		let display = self.build_display();

		// Same approach as Text: padding + wrapping
		let content_width = w.saturating_sub(self.padding_x * 2).max(1);
		let wrapped = rho_text::wrap::wrap_text_with_ansi_str(&display, content_width);

		let left_pad = " ".repeat(self.padding_x);
		let right_pad = " ".repeat(self.padding_x);

		let mut lines = Vec::with_capacity(wrapped.len() + 1);
		// Leading empty line (matches TS behavior)
		lines.push(String::new());

		for line in &wrapped {
			let with_margins = format!("{left_pad}{line}{right_pad}");
			let vis_len = rho_text::width::visible_width_str(&with_margins);
			let pad_needed = w.saturating_sub(vis_len);
			let mut padded = with_margins;
			for _ in 0..pad_needed {
				padded.push(' ');
			}
			lines.push(padded);
		}

		lines
	}
}

/// Cancellable loader — extends Loader with abort signaling.
///
/// Uses an `Arc<AtomicBool>` for abort signaling (Rust equivalent of
/// `AbortController`). Check `is_aborted()` in async work, or clone the
/// `abort_flag()` to pass to tasks.
pub struct CancellableLoader {
	loader:       Loader,
	aborted:      Arc<AtomicBool>,
	/// Called when user presses Escape.
	pub on_abort: Option<Box<dyn FnMut()>>,
}

impl CancellableLoader {
	pub fn new(spinner_color_fn: ColorFn, message_color_fn: ColorFn, message: &str) -> Self {
		Self {
			loader:   Loader::new(spinner_color_fn, message_color_fn, message),
			aborted:  Arc::new(AtomicBool::new(false)),
			on_abort: None,
		}
	}

	/// Get the abort flag (clone for passing to background tasks).
	pub fn abort_flag(&self) -> Arc<AtomicBool> {
		Arc::clone(&self.aborted)
	}

	pub fn is_aborted(&self) -> bool {
		self.aborted.load(Ordering::Relaxed)
	}

	pub const fn loader(&self) -> &Loader {
		&self.loader
	}

	pub const fn loader_mut(&mut self) -> &mut Loader {
		&mut self.loader
	}

	/// Advance animation. Returns true if frame changed.
	pub fn tick(&mut self) -> bool {
		self.loader.tick()
	}
}

impl Component for CancellableLoader {
	fn render(&mut self, width: u16) -> Vec<String> {
		self.loader.render(width)
	}

	fn handle_input(&mut self, data: &str) -> InputResult {
		if crate::keys::match_key::matches_key(data.as_bytes(), "escape", false)
			|| crate::keys::match_key::matches_key(data.as_bytes(), "esc", false)
		{
			self.aborted.store(true, Ordering::Relaxed);
			if let Some(ref mut cb) = self.on_abort {
				cb();
			}
			return InputResult::Consumed;
		}
		InputResult::Ignored
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn identity_fn() -> ColorFn {
		Box::new(|s: &str| s.to_owned())
	}

	#[test]
	fn test_loader_render() {
		let mut loader = Loader::new(identity_fn(), identity_fn(), "Loading...");
		let lines = loader.render(40);
		// Should have leading empty line + content
		assert!(lines.len() >= 2);
		assert_eq!(lines[0], "");
		assert!(lines[1].contains("Loading..."));
	}

	#[test]
	fn test_loader_tick() {
		let mut loader = Loader::new(identity_fn(), identity_fn(), "test");
		// Immediately after creation, tick should not advance (interval not elapsed)
		assert!(!loader.tick());
		// Force time forward
		loader.last_frame_time = Instant::now() - FRAME_INTERVAL;
		assert!(loader.tick());
		assert_eq!(loader.current_frame, 1);
	}

	#[test]
	fn test_loader_stop_start() {
		let mut loader = Loader::new(identity_fn(), identity_fn(), "test");
		assert!(loader.is_running());
		loader.stop();
		assert!(!loader.is_running());
		loader.last_frame_time = Instant::now() - FRAME_INTERVAL;
		assert!(!loader.tick()); // stopped
		loader.start();
		assert!(loader.is_running());
	}

	#[test]
	fn test_cancellable_loader_escape() {
		let mut loader = CancellableLoader::new(identity_fn(), identity_fn(), "Working...");
		assert!(!loader.is_aborted());
		let result = loader.handle_input("\x1b");
		assert_eq!(result, InputResult::Consumed);
		assert!(loader.is_aborted());
	}

	#[test]
	fn test_cancellable_loader_non_escape_ignored() {
		let mut loader = CancellableLoader::new(identity_fn(), identity_fn(), "Working...");
		let result = loader.handle_input("a");
		assert_eq!(result, InputResult::Ignored);
		assert!(!loader.is_aborted());
	}
}
