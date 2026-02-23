//! ANSI-aware text measurement, slicing, wrapping, and sanitization.
//!
//! Provides both UTF-8 (`&str`) and UTF-16 (`&[u16]`) APIs for terminal text
//! processing. The UTF-16 path exists for N-API/JS interop backward
//! compatibility; new Rust consumers should prefer the `&str` APIs.
//!
//! # Features
//! - Single-pass ANSI scanning (no O(n²) rescans)
//! - ASCII fast-path (no grapheme segmentation, no UTF-8 conversion)
//! - Zero-allocation width measurement with early exit
//! - ANSI state preservation across line breaks and slices

pub mod ansi;
pub mod sanitize;
pub mod slice;
pub mod truncate;
pub mod width;
pub mod wrap;

pub use ansi::AnsiState;
pub use sanitize::{sanitize_text_str, sanitize_text_u16};
pub use slice::{
	ExtractSegmentsResult, SliceResult, extract_segments_str, extract_segments_u16,
	slice_with_width_str, slice_with_width_u16,
};
pub use truncate::{EllipsisKind, truncate_to_width_str, truncate_to_width_u16};
pub use width::{visible_width_str, visible_width_u16};
pub use wrap::{wrap_text_with_ansi_str, wrap_text_with_ansi_u16};

/// Tab width in terminal cells.
pub const TAB_WIDTH: usize = 3;

/// ESC byte value (used in UTF-16 as well).
pub(crate) const ESC_U16: u16 = 0x1b;
pub(crate) const ESC_U8: u8 = 0x1b;
