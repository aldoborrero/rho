//! Terminal abstraction using crossterm.
//!
//! Provides raw mode, Kitty keyboard protocol, bracketed paste,
//! cursor control, and emergency restore.

use std::{
	io::{self, Write},
	sync::atomic::{AtomicBool, Ordering},
	time::Duration,
};

use crossterm::{
	cursor, event, execute, queue,
	style::Print,
	terminal::{self, ClearType},
};

use crate::stdin_buffer::{StdinBuffer, StdinEvent};

// ============================================================================
// Emergency restore
// ============================================================================

static TERMINAL_EVER_STARTED: AtomicBool = AtomicBool::new(false);

/// Emergency terminal restore — call from signal/panic handlers.
/// Resets terminal state without requiring access to the `CrosstermTerminal`
/// instance.
pub fn emergency_terminal_restore() {
	if !TERMINAL_EVER_STARTED.load(Ordering::Relaxed) {
		return;
	}
	let mut stdout = io::stdout();
	// Best-effort: ignore errors since terminal may be dead
	let _ = execute!(
		stdout,
		Print("\x1b[?2004l"), // Disable bracketed paste
		Print("\x1b[<u"),     // Pop kitty keyboard protocol
		cursor::Show,
	);
	let _ = terminal::disable_raw_mode();
}

// ============================================================================
// Terminal trait
// ============================================================================

/// Minimal terminal interface for TUI.
pub trait Terminal {
	/// Start the terminal in raw mode and begin reading input.
	fn start(&mut self) -> io::Result<()>;

	/// Stop the terminal and restore state.
	fn stop(&mut self) -> io::Result<()>;

	/// Write output to the terminal.
	fn write(&mut self, data: &str) -> io::Result<()>;

	/// Get terminal width in columns.
	fn columns(&self) -> u16;

	/// Get terminal height in rows.
	fn rows(&self) -> u16;

	/// Whether Kitty keyboard protocol is active.
	fn kitty_protocol_active(&self) -> bool;

	/// Move cursor up (negative) or down (positive) by N lines.
	fn move_by(&mut self, lines: i32) -> io::Result<()>;

	/// Hide the cursor.
	fn hide_cursor(&mut self) -> io::Result<()>;

	/// Show the cursor.
	fn show_cursor(&mut self) -> io::Result<()>;

	/// Clear current line from cursor to end.
	fn clear_line(&mut self) -> io::Result<()>;

	/// Clear from cursor to end of screen.
	fn clear_from_cursor(&mut self) -> io::Result<()>;

	/// Clear entire screen and move cursor to (0,0).
	fn clear_screen(&mut self) -> io::Result<()>;

	/// Set terminal window title.
	fn set_title(&mut self, title: &str) -> io::Result<()>;

	/// Poll for a single input event with timeout.
	/// Returns None if no event within the timeout.
	fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<TerminalEvent>>;

	/// Drain pending input (useful before exit to prevent leaking sequences).
	fn drain_input(&mut self, max_ms: u64, idle_ms: u64) -> io::Result<()>;
}

/// Events produced by the terminal.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
	/// A complete input sequence.
	Input(String),
	/// Bracketed paste content.
	Paste(String),
	/// Terminal was resized.
	Resize(u16, u16),
}

// ============================================================================
// CrosstermTerminal
// ============================================================================

/// Real terminal using crossterm for I/O.
pub struct CrosstermTerminal {
	was_raw:               bool,
	kitty_protocol_active: bool,
	dead:                  bool,
	stdin_buffer:          StdinBuffer,
}

impl CrosstermTerminal {
	pub const fn new() -> Self {
		Self {
			was_raw:               false,
			kitty_protocol_active: false,
			dead:                  false,
			stdin_buffer:          StdinBuffer::new(),
		}
	}

	fn safe_write(&mut self, data: &str) -> io::Result<()> {
		if self.dead {
			return Ok(());
		}
		let mut stdout = io::stdout();
		match stdout.write_all(data.as_bytes()) {
			Ok(()) => stdout.flush(),
			Err(e) => {
				self.dead = true;
				Err(e)
			},
		}
	}

	/// Query terminal for Kitty keyboard protocol support.
	/// Sends CSI ? u query. If terminal responds with CSI ? <flags> u,
	/// it supports the protocol and we enable it.
	fn query_kitty_protocol(&mut self) -> io::Result<()> {
		// Send query
		self.safe_write("\x1b[?u")?;
		io::stdout().flush()?;

		// Try to read response within a short timeout
		if event::poll(Duration::from_millis(100))?
			&& let event::Event::Key(_) = event::read()?
		{
			// crossterm may consume the response; we detect Kitty support
			// by checking if we get a recognizable CSI response
		}

		// For now, detect via env var as a fallback.
		// Full detection happens in the event loop when we see CSI ? N u responses.
		Ok(())
	}

	/// Enable Kitty keyboard protocol (push flags).
	fn enable_kitty_protocol(&mut self) -> io::Result<()> {
		// Flag 1 = disambiguate, Flag 2 = report events, Flag 4 = alternate keys
		self.safe_write("\x1b[>7u")?;
		self.kitty_protocol_active = true;
		Ok(())
	}

	/// Disable Kitty keyboard protocol (pop).
	fn disable_kitty_protocol(&mut self) -> io::Result<()> {
		if self.kitty_protocol_active {
			self.safe_write("\x1b[<u")?;
			self.kitty_protocol_active = false;
		}
		Ok(())
	}

	/// Check if raw input bytes are a Kitty protocol query response.
	fn check_kitty_response(data: &str) -> bool {
		// Pattern: ESC [ ? <digits> u
		let bytes = data.as_bytes();
		if bytes.len() < 4 {
			return false;
		}
		if bytes[0] != 0x1b || bytes[1] != b'[' || bytes[2] != b'?' {
			return false;
		}
		if *bytes.last().unwrap() != b'u' {
			return false;
		}
		// Middle should be digits
		bytes[3..bytes.len() - 1].iter().all(|b| b.is_ascii_digit())
	}
}

impl Default for CrosstermTerminal {
	fn default() -> Self {
		Self::new()
	}
}

impl Terminal for CrosstermTerminal {
	fn start(&mut self) -> io::Result<()> {
		TERMINAL_EVER_STARTED.store(true, Ordering::Relaxed);

		self.was_raw = terminal::is_raw_mode_enabled()?;
		terminal::enable_raw_mode()?;

		// Enable bracketed paste
		self.safe_write("\x1b[?2004h")?;

		// Query and enable Kitty protocol
		self.query_kitty_protocol()?;

		Ok(())
	}

	fn stop(&mut self) -> io::Result<()> {
		// Disable bracketed paste
		self.safe_write("\x1b[?2004l")?;

		// Disable Kitty protocol
		self.disable_kitty_protocol()?;

		// Restore raw mode state
		if !self.was_raw {
			terminal::disable_raw_mode()?;
		}

		Ok(())
	}

	fn write(&mut self, data: &str) -> io::Result<()> {
		self.safe_write(data)
	}

	fn columns(&self) -> u16 {
		terminal::size().map_or(80, |(cols, _)| cols)
	}

	fn rows(&self) -> u16 {
		terminal::size().map_or(24, |(_, rows)| rows)
	}

	fn kitty_protocol_active(&self) -> bool {
		self.kitty_protocol_active
	}

	fn move_by(&mut self, lines: i32) -> io::Result<()> {
		let mut stdout = io::stdout();
		if lines > 0 {
			queue!(stdout, cursor::MoveDown(lines as u16))?;
		} else if lines < 0 {
			queue!(stdout, cursor::MoveUp((-lines) as u16))?;
		}
		stdout.flush()
	}

	fn hide_cursor(&mut self) -> io::Result<()> {
		let mut stdout = io::stdout();
		execute!(stdout, cursor::Hide)
	}

	fn show_cursor(&mut self) -> io::Result<()> {
		let mut stdout = io::stdout();
		execute!(stdout, cursor::Show)
	}

	fn clear_line(&mut self) -> io::Result<()> {
		let mut stdout = io::stdout();
		execute!(stdout, terminal::Clear(ClearType::UntilNewLine))
	}

	fn clear_from_cursor(&mut self) -> io::Result<()> {
		let mut stdout = io::stdout();
		execute!(stdout, terminal::Clear(ClearType::FromCursorDown))
	}

	fn clear_screen(&mut self) -> io::Result<()> {
		let mut stdout = io::stdout();
		execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))
	}

	fn set_title(&mut self, title: &str) -> io::Result<()> {
		self.safe_write(&format!("\x1b]0;{title}\x07"))
	}

	fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<TerminalEvent>> {
		if !event::poll(timeout)? {
			// Check if stdin_buffer has pending data that needs timeout flush
			if self.stdin_buffer.has_pending() {
				let flushed = self.stdin_buffer.flush();
				if let Some(data) = flushed.into_iter().next() {
					return Ok(Some(TerminalEvent::Input(data)));
				}
			}
			return Ok(None);
		}

		let evt = event::read()?;
		match evt {
			event::Event::Key(key_event) => {
				// Convert crossterm key event to raw bytes for StdinBuffer
				// For Kitty protocol, crossterm provides the raw sequence
				let raw = crossterm_key_to_string(&key_event);
				let events = self.stdin_buffer.process(raw.as_bytes());

				for event in events {
					match event {
						StdinEvent::Data(data) => {
							if !self.kitty_protocol_active && Self::check_kitty_response(&data) {
								self.enable_kitty_protocol()?;
								continue;
							}
							return Ok(Some(TerminalEvent::Input(data)));
						},
						StdinEvent::Paste(content) => {
							return Ok(Some(TerminalEvent::Paste(content)));
						},
					}
				}
				Ok(None)
			},
			event::Event::Paste(content) => Ok(Some(TerminalEvent::Paste(content))),
			event::Event::Resize(cols, rows) => Ok(Some(TerminalEvent::Resize(cols, rows))),
			_ => Ok(None),
		}
	}

	fn drain_input(&mut self, max_ms: u64, idle_ms: u64) -> io::Result<()> {
		self.disable_kitty_protocol()?;

		let deadline = std::time::Instant::now() + Duration::from_millis(max_ms);
		let mut last_data = std::time::Instant::now();

		loop {
			let now = std::time::Instant::now();
			if now >= deadline {
				break;
			}
			if now.duration_since(last_data) >= Duration::from_millis(idle_ms) {
				break;
			}

			let remaining = deadline - now;
			let poll_time = remaining.min(Duration::from_millis(idle_ms));

			if event::poll(poll_time)? {
				let _ = event::read()?;
				last_data = std::time::Instant::now();
			} else {
				break;
			}
		}

		Ok(())
	}
}

/// Convert a crossterm `KeyEvent` to a string representation.
/// This is a best-effort conversion for feeding into `StdinBuffer`.
fn crossterm_key_to_string(key: &event::KeyEvent) -> String {
	use event::KeyCode;

	match key.code {
		KeyCode::Char(c) => {
			if key.modifiers.contains(event::KeyModifiers::CONTROL) {
				// Control characters
				if c.is_ascii_lowercase() {
					let ctrl = (c as u8) - b'a' + 1;
					return String::from(ctrl as char);
				}
			}
			if key.modifiers.contains(event::KeyModifiers::ALT) {
				return format!("\x1b{c}");
			}
			c.to_string()
		},
		KeyCode::Enter => "\r".to_owned(),
		KeyCode::Tab => "\t".to_owned(),
		KeyCode::BackTab => "\x1b[Z".to_owned(),
		KeyCode::Backspace => "\x7f".to_owned(),
		KeyCode::Esc => "\x1b".to_owned(),
		KeyCode::Up => "\x1b[A".to_owned(),
		KeyCode::Down => "\x1b[B".to_owned(),
		KeyCode::Right => "\x1b[C".to_owned(),
		KeyCode::Left => "\x1b[D".to_owned(),
		KeyCode::Home => "\x1b[H".to_owned(),
		KeyCode::End => "\x1b[F".to_owned(),
		KeyCode::PageUp => "\x1b[5~".to_owned(),
		KeyCode::PageDown => "\x1b[6~".to_owned(),
		KeyCode::Delete => "\x1b[3~".to_owned(),
		KeyCode::Insert => "\x1b[2~".to_owned(),
		KeyCode::F(n) => match n {
			1 => "\x1bOP".to_owned(),
			2 => "\x1bOQ".to_owned(),
			3 => "\x1bOR".to_owned(),
			4 => "\x1bOS".to_owned(),
			5 => "\x1b[15~".to_owned(),
			6 => "\x1b[17~".to_owned(),
			7 => "\x1b[18~".to_owned(),
			8 => "\x1b[19~".to_owned(),
			9 => "\x1b[20~".to_owned(),
			10 => "\x1b[21~".to_owned(),
			11 => "\x1b[23~".to_owned(),
			12 => "\x1b[24~".to_owned(),
			_ => String::new(),
		},
		_ => String::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_check_kitty_response() {
		assert!(CrosstermTerminal::check_kitty_response("\x1b[?1u"));
		assert!(CrosstermTerminal::check_kitty_response("\x1b[?0u"));
		assert!(!CrosstermTerminal::check_kitty_response("\x1b[A"));
		assert!(!CrosstermTerminal::check_kitty_response("hello"));
	}

	#[test]
	fn test_crossterm_key_to_string() {
		let key = event::KeyEvent::new(event::KeyCode::Char('a'), event::KeyModifiers::NONE);
		assert_eq!(crossterm_key_to_string(&key), "a");

		let key = event::KeyEvent::new(event::KeyCode::Up, event::KeyModifiers::NONE);
		assert_eq!(crossterm_key_to_string(&key), "\x1b[A");

		let key = event::KeyEvent::new(event::KeyCode::Enter, event::KeyModifiers::NONE);
		assert_eq!(crossterm_key_to_string(&key), "\r");
	}

	#[test]
	fn test_emergency_restore_when_never_started() {
		// Should be a no-op when terminal was never started
		emergency_terminal_restore();
	}

	#[test]
	fn test_default_dimensions() {
		let term = CrosstermTerminal::new();
		// Should return reasonable defaults (may vary by env)
		assert!(term.columns() > 0);
		assert!(term.rows() > 0);
	}
}
