//! Overlay positioning and layout resolution.
//!
//! Overlays are modal components rendered on top of the base content.
//! This module handles anchor-based positioning, percentage sizing,
//! margin constraints, and visibility callbacks.

use std::cmp::{max, min};

/// Anchor position for overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayAnchor {
	#[default]
	Center,
	TopLeft,
	TopRight,
	BottomLeft,
	BottomRight,
	TopCenter,
	BottomCenter,
	LeftCenter,
	RightCenter,
}

/// Margin configuration for overlays.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverlayMargin {
	pub top:    u16,
	pub right:  u16,
	pub bottom: u16,
	pub left:   u16,
}

impl OverlayMargin {
	/// Create uniform margin on all sides.
	pub const fn uniform(n: u16) -> Self {
		Self { top: n, right: n, bottom: n, left: n }
	}
}

/// Value that can be absolute or percentage.
#[derive(Debug, Clone, Copy)]
pub enum SizeValue {
	Absolute(u16),
	Percent(f32),
}

impl SizeValue {
	/// Resolve to an absolute value given a reference size.
	fn resolve(self, reference: u16) -> u16 {
		match self {
			Self::Absolute(v) => v,
			Self::Percent(p) => (f32::from(reference) * p / 100.0).floor() as u16,
		}
	}
}

/// Options for overlay positioning and sizing.
#[derive(Debug, Clone, Default)]
pub struct OverlayOptions {
	/// Width in columns or percentage of terminal width.
	pub width:      Option<SizeValue>,
	/// Minimum width in columns.
	pub min_width:  Option<u16>,
	/// Maximum height in rows or percentage of terminal height.
	pub max_height: Option<SizeValue>,

	/// Anchor point for positioning (default: Center).
	pub anchor:   OverlayAnchor,
	/// Horizontal offset from anchor position (positive = right).
	pub offset_x: i16,
	/// Vertical offset from anchor position (positive = down).
	pub offset_y: i16,

	/// Row position: absolute or percentage.
	pub row: Option<SizeValue>,
	/// Column position: absolute or percentage.
	pub col: Option<SizeValue>,

	/// Margin from terminal edges.
	pub margin: OverlayMargin,
}

/// Resolved overlay layout.
#[derive(Debug, Clone, Copy)]
pub struct OverlayLayout {
	pub width:      u16,
	pub row:        u16,
	pub col:        u16,
	pub max_height: Option<u16>,
}

/// Resolve overlay layout from options.
pub fn resolve_overlay_layout(
	options: &OverlayOptions,
	overlay_height: u16,
	term_width: u16,
	term_height: u16,
) -> OverlayLayout {
	let margin_top = options.margin.top;
	let margin_right = options.margin.right;
	let margin_bottom = options.margin.bottom;
	let margin_left = options.margin.left;

	// Available space after margins
	let avail_width = term_width.saturating_sub(margin_left + margin_right).max(1);
	let avail_height = term_height
		.saturating_sub(margin_top + margin_bottom)
		.max(1);

	// === Resolve width ===
	let mut width = options
		.width
		.map_or_else(|| min(80, avail_width), |sv| sv.resolve(term_width));
	if let Some(min_w) = options.min_width {
		width = max(width, min_w);
	}
	width = width.clamp(1, avail_width);

	// === Resolve maxHeight ===
	let max_height = options.max_height.map(|sv| {
		let h = sv.resolve(term_height);
		h.clamp(1, avail_height)
	});

	// Effective overlay height
	let effective_height = max_height.map_or(overlay_height, |mh| min(overlay_height, mh));

	// === Resolve position ===
	let row = if let Some(sv) = options.row {
		match sv {
			SizeValue::Percent(p) => {
				let max_row = avail_height.saturating_sub(effective_height);
				let offset = (f32::from(max_row) * p / 100.0).floor() as u16;
				margin_top + offset
			},
			SizeValue::Absolute(v) => v,
		}
	} else {
		resolve_anchor_row(options.anchor, effective_height, avail_height, margin_top)
	};

	let col = if let Some(sv) = options.col {
		match sv {
			SizeValue::Percent(p) => {
				let max_col = avail_width.saturating_sub(width);
				let offset = (f32::from(max_col) * p / 100.0).floor() as u16;
				margin_left + offset
			},
			SizeValue::Absolute(v) => v,
		}
	} else {
		resolve_anchor_col(options.anchor, width, avail_width, margin_left)
	};

	// Apply offsets
	let row = (row as i32 + i32::from(options.offset_y)).max(0) as u16;
	let col = (col as i32 + i32::from(options.offset_x)).max(0) as u16;

	// Clamp to terminal bounds (respecting margins)
	let max_row = term_height
		.saturating_sub(margin_bottom)
		.saturating_sub(effective_height);
	let max_col = term_width
		.saturating_sub(margin_right)
		.saturating_sub(width);
	let row = row.clamp(margin_top, max_row);
	let col = col.clamp(margin_left, max_col);

	OverlayLayout { width, row, col, max_height }
}

const fn resolve_anchor_row(
	anchor: OverlayAnchor,
	height: u16,
	avail_height: u16,
	margin_top: u16,
) -> u16 {
	match anchor {
		OverlayAnchor::TopLeft | OverlayAnchor::TopCenter | OverlayAnchor::TopRight => margin_top,
		OverlayAnchor::BottomLeft | OverlayAnchor::BottomCenter | OverlayAnchor::BottomRight => {
			margin_top + avail_height.saturating_sub(height)
		},
		OverlayAnchor::LeftCenter | OverlayAnchor::Center | OverlayAnchor::RightCenter => {
			margin_top + avail_height.saturating_sub(height) / 2
		},
	}
}

const fn resolve_anchor_col(
	anchor: OverlayAnchor,
	width: u16,
	avail_width: u16,
	margin_left: u16,
) -> u16 {
	match anchor {
		OverlayAnchor::TopLeft | OverlayAnchor::LeftCenter | OverlayAnchor::BottomLeft => margin_left,
		OverlayAnchor::TopRight | OverlayAnchor::RightCenter | OverlayAnchor::BottomRight => {
			margin_left + avail_width.saturating_sub(width)
		},
		OverlayAnchor::TopCenter | OverlayAnchor::Center | OverlayAnchor::BottomCenter => {
			margin_left + avail_width.saturating_sub(width) / 2
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_center_overlay() {
		let opts = OverlayOptions::default(); // anchor: Center
		let layout = resolve_overlay_layout(&opts, 10, 80, 24);
		// Width defaults to min(80, avail_width) = 80
		assert_eq!(layout.width, 80);
		// Row: center = (24 - 10) / 2 = 7
		assert_eq!(layout.row, 7);
	}

	#[test]
	fn test_top_left_overlay() {
		let opts = OverlayOptions {
			anchor: OverlayAnchor::TopLeft,
			width: Some(SizeValue::Absolute(40)),
			..Default::default()
		};
		let layout = resolve_overlay_layout(&opts, 5, 80, 24);
		assert_eq!(layout.width, 40);
		assert_eq!(layout.row, 0);
		assert_eq!(layout.col, 0);
	}

	#[test]
	fn test_bottom_right_with_margin() {
		let opts = OverlayOptions {
			anchor: OverlayAnchor::BottomRight,
			width: Some(SizeValue::Absolute(20)),
			margin: OverlayMargin { top: 2, right: 3, bottom: 2, left: 3 },
			..Default::default()
		};
		let layout = resolve_overlay_layout(&opts, 5, 80, 24);
		assert_eq!(layout.width, 20);
		// Row: margin_top + avail_height - height = 2 + (24-2-2) - 5 = 2 + 15 = 17
		assert_eq!(layout.row, 17);
		// Col: margin_left + avail_width - width = 3 + (80-3-3) - 20 = 3 + 54 = 57
		assert_eq!(layout.col, 57);
	}

	#[test]
	fn test_percentage_width() {
		let opts = OverlayOptions { width: Some(SizeValue::Percent(50.0)), ..Default::default() };
		let layout = resolve_overlay_layout(&opts, 5, 80, 24);
		assert_eq!(layout.width, 40);
	}

	#[test]
	fn test_max_height() {
		let opts = OverlayOptions { max_height: Some(SizeValue::Absolute(3)), ..Default::default() };
		let layout = resolve_overlay_layout(&opts, 10, 80, 24);
		assert_eq!(layout.max_height, Some(3));
	}

	#[test]
	fn test_min_width() {
		let opts = OverlayOptions {
			width: Some(SizeValue::Absolute(5)),
			min_width: Some(20),
			..Default::default()
		};
		let layout = resolve_overlay_layout(&opts, 5, 80, 24);
		assert_eq!(layout.width, 20);
	}

	#[test]
	fn test_offset() {
		let opts = OverlayOptions {
			anchor: OverlayAnchor::TopLeft,
			width: Some(SizeValue::Absolute(20)),
			offset_x: 5,
			offset_y: 3,
			..Default::default()
		};
		let layout = resolve_overlay_layout(&opts, 5, 80, 24);
		assert_eq!(layout.row, 3);
		assert_eq!(layout.col, 5);
	}

	#[test]
	fn test_clamp_to_bounds() {
		let opts = OverlayOptions {
			anchor: OverlayAnchor::TopLeft,
			width: Some(SizeValue::Absolute(20)),
			offset_x: 100, // way beyond terminal
			offset_y: 100,
			..Default::default()
		};
		let layout = resolve_overlay_layout(&opts, 5, 80, 24);
		// Should be clamped
		assert!(layout.col + layout.width <= 80);
		assert!(layout.row + 5 <= 24);
	}
}
