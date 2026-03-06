//! ANSI-aware text measurement, slicing, wrapping, and sanitization.
//!
//! Provides UTF-8 (`&str`) APIs for terminal text processing.
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
pub use sanitize::sanitize_text_str;
pub use slice::{ExtractSegmentsResult, SliceResult, extract_segments_str, slice_with_width_str};
pub use truncate::{EllipsisKind, truncate_to_width_str};
pub use width::visible_width_str;
pub use wrap::wrap_text_with_ansi_str;

/// Tab width in terminal cells.
pub const TAB_WIDTH: usize = 3;

pub(crate) const ESC_U8: u8 = 0x1b;
