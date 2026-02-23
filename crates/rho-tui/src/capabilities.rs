//! Terminal capability detection and image rendering.
//!
//! Detects terminal emulator from environment variables and provides
//! image protocol encoding (Kitty, iTerm2), image dimension parsing,
//! and notification formatting.

use std::{env, fmt::Write as _, sync::OnceLock};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

// ============================================================================
// Enums
// ============================================================================

/// Image display protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
	Kitty,
	Iterm2,
}

/// Desktop notification protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyProtocol {
	Bell,
	Osc9,
	Osc99,
}

/// Known terminal identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalId {
	Kitty,
	Ghostty,
	WezTerm,
	Iterm2,
	VsCode,
	Alacritty,
	TrueColor,
	Base,
}

// ============================================================================
// TerminalInfo
// ============================================================================

/// Terminal capability details used for rendering and protocol selection.
#[derive(Debug, Clone)]
pub struct TerminalInfo {
	pub id:              TerminalId,
	pub image_protocol:  Option<ImageProtocol>,
	pub true_color:      bool,
	pub hyperlinks:      bool,
	pub notify_protocol: NotifyProtocol,
}

impl TerminalInfo {
	const fn new(
		id: TerminalId,
		image_protocol: Option<ImageProtocol>,
		true_color: bool,
		hyperlinks: bool,
		notify_protocol: NotifyProtocol,
	) -> Self {
		Self { id, image_protocol, true_color, hyperlinks, notify_protocol }
	}

	/// Check if a rendered line contains an image sequence.
	pub fn is_image_line(&self, line: &str) -> bool {
		let Some(proto) = self.image_protocol else {
			return false;
		};
		let prefix = match proto {
			ImageProtocol::Kitty => "\x1b_G",
			ImageProtocol::Iterm2 => "\x1b]1337;File=",
		};
		// Image escape sequences always appear at the start of a line,
		// so only scan the first 64 bytes. Use floor_char_boundary to
		// avoid panicking on multi-byte UTF-8 characters.
		let end = line.len().min(64);
		let end = line.floor_char_boundary(end);
		line[..end].contains(prefix)
	}

	/// Format a notification message for this terminal.
	pub fn format_notification(&self, message: &str) -> String {
		match self.notify_protocol {
			NotifyProtocol::Bell => "\x07".to_owned(),
			NotifyProtocol::Osc9 => format!("\x1b]9;{message}\x1b\\"),
			NotifyProtocol::Osc99 => format!("\x1b]99;;{message}\x1b\\"),
		}
	}
}

// ============================================================================
// Known terminals
// ============================================================================

const TERMINAL_KITTY: TerminalInfo = TerminalInfo::new(
	TerminalId::Kitty,
	Some(ImageProtocol::Kitty),
	true,
	true,
	NotifyProtocol::Osc99,
);
const TERMINAL_GHOSTTY: TerminalInfo = TerminalInfo::new(
	TerminalId::Ghostty,
	Some(ImageProtocol::Kitty),
	true,
	true,
	NotifyProtocol::Osc9,
);
const TERMINAL_WEZTERM: TerminalInfo = TerminalInfo::new(
	TerminalId::WezTerm,
	Some(ImageProtocol::Kitty),
	true,
	true,
	NotifyProtocol::Osc9,
);
const TERMINAL_ITERM2: TerminalInfo = TerminalInfo::new(
	TerminalId::Iterm2,
	Some(ImageProtocol::Iterm2),
	true,
	true,
	NotifyProtocol::Osc9,
);
const TERMINAL_VSCODE: TerminalInfo =
	TerminalInfo::new(TerminalId::VsCode, None, true, true, NotifyProtocol::Bell);
const TERMINAL_ALACRITTY: TerminalInfo =
	TerminalInfo::new(TerminalId::Alacritty, None, true, true, NotifyProtocol::Bell);
const TERMINAL_TRUECOLOR: TerminalInfo =
	TerminalInfo::new(TerminalId::TrueColor, None, true, true, NotifyProtocol::Bell);
const TERMINAL_BASE: TerminalInfo =
	TerminalInfo::new(TerminalId::Base, None, false, true, NotifyProtocol::Bell);

/// Get terminal info for a given terminal ID.
pub const fn get_terminal_info(id: TerminalId) -> TerminalInfo {
	match id {
		TerminalId::Kitty => TERMINAL_KITTY,
		TerminalId::Ghostty => TERMINAL_GHOSTTY,
		TerminalId::WezTerm => TERMINAL_WEZTERM,
		TerminalId::Iterm2 => TERMINAL_ITERM2,
		TerminalId::VsCode => TERMINAL_VSCODE,
		TerminalId::Alacritty => TERMINAL_ALACRITTY,
		TerminalId::TrueColor => TERMINAL_TRUECOLOR,
		TerminalId::Base => TERMINAL_BASE,
	}
}

// ============================================================================
// Terminal detection
// ============================================================================

/// Detect terminal emulator from environment variables.
pub fn detect_terminal_id() -> TerminalId {
	fn has_env(name: &str) -> bool {
		env::var_os(name).is_some_and(|v| !v.is_empty())
	}

	fn env_eq_ci(name: &str, expected: &str) -> bool {
		env::var(name).is_ok_and(|v| v.eq_ignore_ascii_case(expected))
	}

	fn env_contains_ci(name: &str, needle: &str) -> bool {
		env::var(name).is_ok_and(|v| {
			v.to_ascii_lowercase()
				.contains(&needle.to_ascii_lowercase())
		})
	}

	// Specific env vars
	if has_env("KITTY_WINDOW_ID") {
		return TerminalId::Kitty;
	}
	if has_env("GHOSTTY_RESOURCES_DIR") {
		return TerminalId::Ghostty;
	}
	if has_env("WEZTERM_PANE") {
		return TerminalId::WezTerm;
	}
	if has_env("ITERM_SESSION_ID") {
		return TerminalId::Iterm2;
	}
	if has_env("VSCODE_PID") {
		return TerminalId::VsCode;
	}
	if has_env("ALACRITTY_WINDOW_ID") {
		return TerminalId::Alacritty;
	}

	// TERM_PROGRAM fallback
	if env::var("TERM_PROGRAM").is_ok() {
		if env_eq_ci("TERM_PROGRAM", "kitty") {
			return TerminalId::Kitty;
		}
		if env_eq_ci("TERM_PROGRAM", "ghostty") {
			return TerminalId::Ghostty;
		}
		if env_eq_ci("TERM_PROGRAM", "wezterm") {
			return TerminalId::WezTerm;
		}
		if env_eq_ci("TERM_PROGRAM", "iterm.app") {
			return TerminalId::Iterm2;
		}
		if env_eq_ci("TERM_PROGRAM", "vscode") {
			return TerminalId::VsCode;
		}
		if env_eq_ci("TERM_PROGRAM", "alacritty") {
			return TerminalId::Alacritty;
		}
	}

	// TERM contains ghostty
	if env_contains_ci("TERM", "ghostty") {
		return TerminalId::Ghostty;
	}

	// COLORTERM for true color
	if env_eq_ci("COLORTERM", "truecolor") || env_eq_ci("COLORTERM", "24bit") {
		return TerminalId::TrueColor;
	}

	TerminalId::Base
}

/// Cached detected terminal info.
static DETECTED_TERMINAL: OnceLock<TerminalInfo> = OnceLock::new();

/// Get the detected terminal info (cached).
pub fn terminal() -> &'static TerminalInfo {
	DETECTED_TERMINAL.get_or_init(|| get_terminal_info(detect_terminal_id()))
}

// ============================================================================
// Notification suppression
// ============================================================================

/// Check if notifications are suppressed via `PI_NOTIFICATIONS` env var.
pub fn is_notification_suppressed() -> bool {
	env::var("PI_NOTIFICATIONS").is_ok_and(|v| v == "off" || v == "0" || v == "false")
}

// ============================================================================
// Cell dimensions
// ============================================================================

/// Cell dimensions in pixels.
#[derive(Debug, Clone, Copy)]
pub struct CellDimensions {
	pub width_px:  u32,
	pub height_px: u32,
}

impl Default for CellDimensions {
	fn default() -> Self {
		Self { width_px: 9, height_px: 18 }
	}
}

/// Image dimensions in pixels.
#[derive(Debug, Clone, Copy)]
pub struct ImageDimensions {
	pub width_px:  u32,
	pub height_px: u32,
}

/// Options for rendering images.
#[derive(Debug, Clone, Copy)]
pub struct ImageRenderOptions {
	pub max_width_cells:       u32,
	pub max_height_cells:      u32,
	pub preserve_aspect_ratio: bool,
}

impl Default for ImageRenderOptions {
	fn default() -> Self {
		Self {
			max_width_cells:       80,
			max_height_cells:      u32::MAX,
			preserve_aspect_ratio: true,
		}
	}
}

// ============================================================================
// Image encoding
// ============================================================================

/// Encode image data for Kitty graphics protocol.
pub fn encode_kitty(
	base64_data: &str,
	columns: Option<u32>,
	rows: Option<u32>,
	image_id: Option<u32>,
) -> String {
	const CHUNK_SIZE: usize = 4096;

	let mut params = vec!["a=T".to_owned(), "f=100".to_owned(), "q=2".to_owned()];
	if let Some(c) = columns {
		params.push(format!("c={c}"));
	}
	if let Some(r) = rows {
		params.push(format!("r={r}"));
	}
	if let Some(id) = image_id {
		params.push(format!("i={id}"));
	}

	if base64_data.len() <= CHUNK_SIZE {
		return format!("\x1b_G{};{}\x1b\\", params.join(","), base64_data);
	}

	let mut result = String::new();
	let mut offset = 0;
	let mut is_first = true;

	while offset < base64_data.len() {
		let end = (offset + CHUNK_SIZE).min(base64_data.len());
		let chunk = &base64_data[offset..end];
		let is_last = end >= base64_data.len();

		if is_first {
			let _ = write!(result, "\x1b_G{},m=1;{}\x1b\\", params.join(","), chunk);
			is_first = false;
		} else if is_last {
			let _ = write!(result, "\x1b_Gm=0;{chunk}\x1b\\");
		} else {
			let _ = write!(result, "\x1b_Gm=1;{chunk}\x1b\\");
		}

		offset += CHUNK_SIZE;
	}

	result
}

/// Encode image data for iTerm2 inline image protocol.
pub fn encode_iterm2(
	base64_data: &str,
	width: Option<&str>,
	height: Option<&str>,
	name: Option<&str>,
	preserve_aspect_ratio: bool,
	inline: bool,
) -> String {
	let mut params = vec![format!("inline={}", i32::from(inline))];

	if let Some(w) = width {
		params.push(format!("width={w}"));
	}
	if let Some(h) = height {
		params.push(format!("height={h}"));
	}
	if let Some(n) = name {
		let name_b64 = BASE64.encode(n.as_bytes());
		params.push(format!("name={name_b64}"));
	}
	if !preserve_aspect_ratio {
		params.push("preserveAspectRatio=0".to_owned());
	}

	format!("\x1b]1337;File={}:{}\x07", params.join(";"), base64_data)
}

/// Calculate number of terminal rows an image will occupy.
pub fn calculate_image_rows(
	image_dims: ImageDimensions,
	target_width_cells: u32,
	cell_dims: CellDimensions,
) -> u32 {
	let target_width_px = target_width_cells * cell_dims.width_px;
	let scale = f64::from(target_width_px) / f64::from(image_dims.width_px);
	let scaled_height_px = f64::from(image_dims.height_px) * scale;
	let rows = (scaled_height_px / f64::from(cell_dims.height_px)).ceil() as u32;
	rows.max(1)
}

// ============================================================================
// Image dimension parsing
// ============================================================================

/// Parse PNG dimensions from raw bytes.
pub fn get_png_dimensions(data: &[u8]) -> Option<ImageDimensions> {
	if data.len() < 24 {
		return None;
	}
	// PNG signature: 0x89 P N G
	if data[0] != 0x89 || data[1] != 0x50 || data[2] != 0x4e || data[3] != 0x47 {
		return None;
	}
	let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
	let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
	Some(ImageDimensions { width_px: width, height_px: height })
}

/// Parse JPEG dimensions from raw bytes.
pub fn get_jpeg_dimensions(data: &[u8]) -> Option<ImageDimensions> {
	if data.len() < 2 || data[0] != 0xff || data[1] != 0xd8 {
		return None;
	}

	let mut offset = 2;
	while offset < data.len().saturating_sub(9) {
		if data[offset] != 0xff {
			offset += 1;
			continue;
		}
		let marker = data[offset + 1];
		if (0xc0..=0xc2).contains(&marker) {
			let height = u16::from_be_bytes([data[offset + 5], data[offset + 6]]);
			let width = u16::from_be_bytes([data[offset + 7], data[offset + 8]]);
			return Some(ImageDimensions {
				width_px:  u32::from(width),
				height_px: u32::from(height),
			});
		}
		if offset + 3 >= data.len() {
			return None;
		}
		let length = u16::from_be_bytes([data[offset + 2], data[offset + 3]]);
		if length < 2 {
			return None;
		}
		offset += 2 + usize::from(length);
	}

	None
}

/// Parse GIF dimensions from raw bytes.
pub fn get_gif_dimensions(data: &[u8]) -> Option<ImageDimensions> {
	if data.len() < 10 {
		return None;
	}
	let sig = &data[..6];
	if sig != b"GIF87a" && sig != b"GIF89a" {
		return None;
	}
	let width = u16::from_le_bytes([data[6], data[7]]);
	let height = u16::from_le_bytes([data[8], data[9]]);
	Some(ImageDimensions { width_px: u32::from(width), height_px: u32::from(height) })
}

/// Parse WebP dimensions from raw bytes.
pub fn get_webp_dimensions(data: &[u8]) -> Option<ImageDimensions> {
	if data.len() < 30 {
		return None;
	}
	if &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
		return None;
	}

	let chunk = &data[12..16];
	if chunk == b"VP8 " {
		if data.len() < 30 {
			return None;
		}
		let width = u32::from(u16::from_le_bytes([data[26], data[27]]) & 0x3fff);
		let height = u32::from(u16::from_le_bytes([data[28], data[29]]) & 0x3fff);
		Some(ImageDimensions { width_px: width, height_px: height })
	} else if chunk == b"VP8L" {
		if data.len() < 25 {
			return None;
		}
		let bits = u32::from_le_bytes([data[21], data[22], data[23], data[24]]);
		let width = (bits & 0x3fff) + 1;
		let height = ((bits >> 14) & 0x3fff) + 1;
		Some(ImageDimensions { width_px: width, height_px: height })
	} else if chunk == b"VP8X" {
		if data.len() < 30 {
			return None;
		}
		let width =
			(u32::from(data[24]) | (u32::from(data[25]) << 8) | (u32::from(data[26]) << 16)) + 1;
		let height =
			(u32::from(data[27]) | (u32::from(data[28]) << 8) | (u32::from(data[29]) << 16)) + 1;
		Some(ImageDimensions { width_px: width, height_px: height })
	} else {
		None
	}
}

/// Parse image dimensions from raw bytes by MIME type.
pub fn get_image_dimensions(data: &[u8], mime_type: &str) -> Option<ImageDimensions> {
	match mime_type {
		"image/png" => get_png_dimensions(data),
		"image/jpeg" => get_jpeg_dimensions(data),
		"image/gif" => get_gif_dimensions(data),
		"image/webp" => get_webp_dimensions(data),
		_ => None,
	}
}

/// Render an image using the appropriate terminal protocol.
pub fn render_image(
	terminal_info: &TerminalInfo,
	base64_data: &str,
	image_dims: ImageDimensions,
	cell_dims: CellDimensions,
	options: &ImageRenderOptions,
) -> Option<(String, u32)> {
	let proto = terminal_info.image_protocol?;
	let max_width = options.max_width_cells;
	let rows = calculate_image_rows(image_dims, max_width, cell_dims);

	match proto {
		ImageProtocol::Kitty => {
			let sequence = encode_kitty(base64_data, Some(max_width), Some(rows), None);
			Some((sequence, rows))
		},
		ImageProtocol::Iterm2 => {
			let w = max_width.to_string();
			let sequence = encode_iterm2(
				base64_data,
				Some(&w),
				Some("auto"),
				None,
				options.preserve_aspect_ratio,
				true,
			);
			Some((sequence, rows))
		},
	}
}

/// Generate fallback text for an image that can't be displayed inline.
pub fn image_fallback(
	mime_type: &str,
	dimensions: Option<ImageDimensions>,
	filename: Option<&str>,
) -> String {
	let mut parts = Vec::new();
	if let Some(name) = filename {
		parts.push(name.to_owned());
	}
	parts.push(format!("[{mime_type}]"));
	if let Some(dims) = dimensions {
		parts.push(format!("{}x{}", dims.width_px, dims.height_px));
	}
	format!("[Image: {}]", parts.join(" "))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_calculate_image_rows() {
		let dims = ImageDimensions { width_px: 200, height_px: 100 };
		let cell = CellDimensions { width_px: 10, height_px: 20 };
		// 10 cells wide * 10px = 100px target width
		// scale = 100/200 = 0.5
		// scaled height = 100 * 0.5 = 50px
		// rows = ceil(50/20) = 3
		assert_eq!(calculate_image_rows(dims, 10, cell), 3);
	}

	#[test]
	fn test_calculate_image_rows_minimum_one() {
		let dims = ImageDimensions { width_px: 1000, height_px: 1 };
		let cell = CellDimensions::default();
		assert!(calculate_image_rows(dims, 10, cell) >= 1);
	}

	#[test]
	fn test_encode_kitty_single_chunk() {
		let data = "AAAA"; // small
		let result = encode_kitty(data, Some(10), Some(5), None);
		assert!(result.starts_with("\x1b_G"));
		assert!(result.contains("a=T"));
		assert!(result.contains("c=10"));
		assert!(result.contains("r=5"));
		assert!(result.ends_with("\x1b\\"));
	}

	#[test]
	fn test_encode_kitty_multi_chunk() {
		let data = "A".repeat(5000);
		let result = encode_kitty(&data, None, None, None);
		// Should have multiple chunks
		assert!(result.contains("m=1"));
		assert!(result.contains("m=0"));
	}

	#[test]
	fn test_encode_iterm2() {
		let result = encode_iterm2("AAAA", Some("80"), Some("auto"), None, true, true);
		assert!(result.starts_with("\x1b]1337;File="));
		assert!(result.contains("inline=1"));
		assert!(result.contains("width=80"));
		assert!(result.ends_with("\x07"));
	}

	#[test]
	fn test_get_png_dimensions() {
		// Minimal PNG-like header
		let mut data = vec![0x89, 0x50, 0x4e, 0x47]; // signature
		data.extend_from_slice(&[0; 12]); // IHDR chunk header
		data.extend_from_slice(&100u32.to_be_bytes()); // width
		data.extend_from_slice(&50u32.to_be_bytes()); // height
		let dims = get_png_dimensions(&data).unwrap();
		assert_eq!(dims.width_px, 100);
		assert_eq!(dims.height_px, 50);
	}

	#[test]
	fn test_get_png_too_short() {
		assert!(get_png_dimensions(&[0x89, 0x50]).is_none());
	}

	#[test]
	fn test_get_gif_dimensions() {
		let mut data = b"GIF89a".to_vec();
		data.extend_from_slice(&320u16.to_le_bytes());
		data.extend_from_slice(&240u16.to_le_bytes());
		let dims = get_gif_dimensions(&data).unwrap();
		assert_eq!(dims.width_px, 320);
		assert_eq!(dims.height_px, 240);
	}

	#[test]
	fn test_image_fallback() {
		let text = image_fallback(
			"image/png",
			Some(ImageDimensions { width_px: 100, height_px: 50 }),
			Some("test.png"),
		);
		assert_eq!(text, "[Image: test.png [image/png] 100x50]");
	}

	#[test]
	fn test_notification_format() {
		let info = get_terminal_info(TerminalId::Kitty);
		let notif = info.format_notification("test");
		assert!(notif.contains("99"));
		assert!(notif.contains("test"));

		let info = get_terminal_info(TerminalId::Base);
		let notif = info.format_notification("test");
		assert_eq!(notif, "\x07");
	}
}
