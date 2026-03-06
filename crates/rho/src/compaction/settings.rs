//! Compaction settings and threshold calculation.
//!
//! oh-my-pi ref: `compaction.ts` `CompactionSettings`,
//! `effectiveReserveTokens()`, `shouldCompact()`

// Re-export the canonical CompactionSettings from the layered settings system.
pub use crate::settings::CompactionSettings;

/// Calculate effective reserve tokens: `max(15% of window, reserve_tokens)`.
///
/// oh-my-pi ref: `compaction.ts` `effectiveReserveTokens()`
#[allow(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "15% of a u32 context window always fits in u32"
)]
pub fn effective_reserve_tokens(context_window: u32, settings: &CompactionSettings) -> u32 {
	let fifteen_percent = (f64::from(context_window) * 0.15) as u32;
	fifteen_percent.max(settings.reserve_tokens)
}

/// Check if compaction should trigger based on used tokens vs window.
///
/// oh-my-pi ref: `compaction.ts` `shouldCompact()`
pub fn should_compact(
	context_tokens: u32,
	context_window: u32,
	settings: &CompactionSettings,
) -> bool {
	if !settings.enabled {
		return false;
	}
	let threshold =
		context_window.saturating_sub(effective_reserve_tokens(context_window, settings));
	context_tokens > threshold
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_settings() {
		let s = CompactionSettings::default();
		assert!(s.enabled);
		assert_eq!(s.reserve_tokens, 16_384);
		assert_eq!(s.keep_recent_tokens, 20_000);
	}

	#[test]
	fn effective_reserve_uses_fifteen_percent() {
		let settings = CompactionSettings { reserve_tokens: 1_000, ..Default::default() };
		// 200,000 * 0.15 = 30,000. max(30_000, 1_000) = 30_000
		assert_eq!(effective_reserve_tokens(200_000, &settings), 30_000);
	}

	#[test]
	fn effective_reserve_uses_setting_when_larger() {
		let settings = CompactionSettings { reserve_tokens: 50_000, ..Default::default() };
		// 200,000 * 0.15 = 30,000. max(30_000, 50_000) = 50_000
		assert_eq!(effective_reserve_tokens(200_000, &settings), 50_000);
	}

	#[test]
	fn should_compact_above_threshold() {
		let settings = CompactionSettings::default();
		// threshold = 200_000 - 30_000 = 170_000
		assert!(should_compact(170_001, 200_000, &settings));
	}

	#[test]
	fn should_compact_below_threshold() {
		let settings = CompactionSettings::default();
		assert!(!should_compact(100_000, 200_000, &settings));
	}

	#[test]
	fn should_compact_disabled() {
		let settings = CompactionSettings { enabled: false, ..Default::default() };
		assert!(!should_compact(200_000, 200_000, &settings));
	}

	#[test]
	fn should_compact_at_exactly_threshold() {
		let settings = CompactionSettings::default();
		// threshold = 200_000 - 30_000 = 170_000
		assert!(!should_compact(170_000, 200_000, &settings));
	}
}
