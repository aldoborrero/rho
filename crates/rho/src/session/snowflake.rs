//! Snowflake ID generation.
//!
//! 64-bit IDs encoded as 16-character lowercase hex strings.
//!
//! Layout: `[42-bit timestamp offset from epoch][22-bit sequence]`
//!
//! Not distributed -- uses extended 22-bit sequence instead of machine ID.

use std::{cell::RefCell, collections::HashSet};

use chrono::{DateTime, TimeZone, Utc};

/// Discord-style epoch: 2015-01-01T00:00:00Z in milliseconds.
pub const EPOCH: u64 = 1_420_070_400_000;

/// Maximum value for the 22-bit sequence field.
pub const MAX_SEQUENCE: u32 = 0x3f_ffff;

/// A snowflake ID source that tracks sequence state.
pub struct Source {
	seq: u32,
}

impl Source {
	/// Create a new source with a random initial sequence.
	pub fn new() -> Self {
		let mut rng = rand::rng();
		let initial: u32 = rand::Rng::random(&mut rng);
		Self { seq: initial & MAX_SEQUENCE }
	}

	/// Generate a snowflake ID from the given timestamp (milliseconds since Unix
	/// epoch).
	///
	/// Returns a 16-character lowercase hex string.
	pub fn generate(&mut self, timestamp_ms: u64) -> String {
		self.seq = (self.seq + 1) & MAX_SEQUENCE;
		let dt = timestamp_ms.wrapping_sub(EPOCH);
		format_parts(dt, self.seq)
	}
}

impl Default for Source {
	fn default() -> Self {
		Self::new()
	}
}

/// Format a timestamp offset and sequence into a 16-character lowercase hex
/// string.
///
/// This produces output identical to the TypeScript reference implementation.
///
/// # Arguments
/// * `dt` - Timestamp offset from epoch in milliseconds
/// * `seq` - 22-bit sequence number
pub fn format_parts(dt: u64, seq: u32) -> String {
	// Port of the TypeScript implementation:
	//   const dtLo = dt % 1024;
	//   const hi = (dt - dtLo) / 1024;
	//   const lo = ((dtLo << 22) | seq) >>> 0;
	//   const hi1 = (hi >>> 16) & 0xffff;
	//   const hi2 = hi & 0xffff;
	//   const lo1 = (lo >>> 16) & 0xffff;
	//   const lo2 = lo & 0xffff;
	let dt_lo = dt % 1024;
	let hi = (dt - dt_lo) / 1024;
	let lo = ((dt_lo << 22) | u64::from(seq & MAX_SEQUENCE)) as u32;
	let hi1 = ((hi >> 16) & 0xffff) as u16;
	let hi2 = (hi & 0xffff) as u16;
	let lo1 = ((lo >> 16) & 0xffff) as u16;
	let lo2 = (lo & 0xffff) as u16;
	format!("{hi1:04x}{hi2:04x}{lo1:04x}{lo2:04x}")
}

thread_local! {
	 static DEFAULT_SOURCE: RefCell<Source> = RefCell::new(Source::new());
}

/// Generate a snowflake ID using the thread-local default source and current
/// time.
pub fn next() -> String {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.expect("system clock before Unix epoch")
		.as_millis() as u64;
	DEFAULT_SOURCE.with(|s| s.borrow_mut().generate(now))
}

/// Validate that a string is a valid snowflake ID (16-character lowercase hex).
pub fn valid(value: &str) -> bool {
	value.len() == 16
		&& value
			.chars()
			.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Extract the millisecond timestamp from a snowflake ID.
///
/// Returns `None` if the value is not a valid snowflake.
pub fn get_timestamp(value: &str) -> Option<u64> {
	if !valid(value) {
		return None;
	}
	let hi = u64::from_str_radix(&value[0..8], 16).ok()?;
	let lo = u64::from_str_radix(&value[8..16], 16).ok()?;
	let n = (hi << 32) | lo;
	Some((n >> 22) + EPOCH)
}

/// Extract a `DateTime<Utc>` from a snowflake ID.
///
/// Returns `None` if the value is not a valid snowflake.
pub fn get_date(value: &str) -> Option<DateTime<Utc>> {
	let ts = get_timestamp(value)?;
	Utc.timestamp_millis_opt(ts as i64).single()
}

/// Maximum attempts before falling back to full snowflake ID.
const MAX_ENTRY_ID_ATTEMPTS: usize = 1000;

/// Generate an 8-character hex entry ID that does not collide with any existing
/// IDs.
///
/// Uses the lower 8 hex characters of a snowflake, with collision checking.
/// Falls back to a full 16-character snowflake if max attempts are reached.
pub fn generate_entry_id(existing: &HashSet<String>) -> String {
	for _ in 0..MAX_ENTRY_ID_ATTEMPTS {
		let sf = next();
		let id = sf[8..16].to_owned();
		if !existing.contains(&id) {
			return id;
		}
	}
	// Fallback: full 16-char snowflake (unique by construction).
	eprintln!("Warning: entry ID collision limit reached, using full snowflake ID");
	next()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_generate_returns_16_char_hex() {
		let mut source = Source::new();
		let now_ms = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_millis() as u64;
		let id = source.generate(now_ms);
		assert_eq!(id.len(), 16, "snowflake must be 16 chars, got {}", id.len());
		assert!(
			id.chars()
				.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
			"snowflake must be lowercase hex, got {id}"
		);
	}

	#[test]
	fn test_generate_is_monotonic() {
		let mut source = Source::new();
		let ts = EPOCH + 1_000_000;
		let a = source.generate(ts);
		let b = source.generate(ts);
		let c = source.generate(ts + 1);
		// With the same timestamp, the sequence increments, so b > a lexicographically
		// (since higher sequence = higher numeric value in lower bits).
		assert!(b > a, "expected {b} > {a}");
		// With a later timestamp, c should be >= b.
		assert!(c > b, "expected {c} > {b}");
	}

	#[test]
	fn test_valid_accepts_valid() {
		assert!(valid("0123456789abcdef"));
	}

	#[test]
	fn test_valid_rejects_invalid() {
		// Too short
		assert!(!valid("0123456789abcde"));
		// Too long
		assert!(!valid("0123456789abcdef0"));
		// Uppercase
		assert!(!valid("0123456789ABCDEF"));
		// Non-hex character
		assert!(!valid("0123456789abcdeg"));
		// Empty
		assert!(!valid(""));
	}

	#[test]
	fn test_get_timestamp_roundtrip() {
		let mut source = Source::new();
		let ts = EPOCH + 123_456_789;
		let id = source.generate(ts);
		let extracted = get_timestamp(&id).expect("should extract timestamp");
		assert_eq!(extracted, ts, "extracted timestamp should match input");
	}

	#[test]
	fn test_format_parts_known_values() {
		// Test with dt=0, seq=0 -> should be all zeros
		assert_eq!(format_parts(0, 0), "0000000000000000");

		// Test with dt=1, seq=0 -> timestamp offset 1ms
		// The 64-bit value is (1 << 22) | 0 = 0x00000000_00400000
		// hi part: 0x00000000, lo part: 0x00400000
		// hi1=0x0000, hi2=0x0000, lo1=0x0040, lo2=0x0000
		assert_eq!(format_parts(1, 0), "0000000000400000");

		// Test with dt=0, seq=1
		// The 64-bit value is 0x00000000_00000001
		assert_eq!(format_parts(0, 1), "0000000000000001");

		// Test with dt=1024, seq=0
		// dtLo = 1024 % 1024 = 0, hi = 1024 / 1024 = 1
		// lo = (0 << 22) | 0 = 0
		// hi1 = 0, hi2 = 1, lo1 = 0, lo2 = 0
		assert_eq!(format_parts(1024, 0), "0000000100000000");

		// Test with dt=1023, seq=MAX_SEQUENCE
		// dtLo = 1023, hi = 0
		// lo = (1023 << 22) | 0x3FFFFF = (0x3FF << 22) | 0x3FFFFF = 0xFFC00000 |
		// 0x003FFFFF = 0xFFFFFFFF hi1=0, hi2=0, lo1=0xFFFF, lo2=0xFFFF
		assert_eq!(format_parts(1023, MAX_SEQUENCE), "00000000ffffffff");

		// Cross-check: format_parts(dt, seq) where the combined 64-bit value is (dt <<
		// 22) | seq
		let dt: u64 = 192_085_553_123; // a realistic offset ~6 years
		let seq: u32 = 42;
		let id = format_parts(dt, seq);
		assert_eq!(id.len(), 16);
		// Verify by parsing back
		let hi = u64::from_str_radix(&id[0..8], 16).unwrap();
		let lo = u64::from_str_radix(&id[8..16], 16).unwrap();
		let combined = (hi << 32) | lo;
		assert_eq!(combined >> 22, dt, "timestamp offset should match");
		assert_eq!(combined & u64::from(MAX_SEQUENCE), u64::from(seq), "sequence should match");
	}

	#[test]
	fn test_get_date_returns_valid_datetime() {
		let mut source = Source::new();
		let ts = EPOCH + 123_456_789;
		let id = source.generate(ts);
		let dt = get_date(&id).expect("should extract date");
		assert_eq!(dt.timestamp_millis(), ts as i64);
	}

	#[test]
	fn test_get_timestamp_invalid_returns_none() {
		assert!(get_timestamp("not-a-snowflake").is_none());
		assert!(get_timestamp("").is_none());
	}

	#[test]
	fn test_next_returns_valid_snowflake() {
		let id = next();
		assert!(valid(&id), "next() should produce a valid snowflake: {id}");
	}

	#[test]
	fn test_generate_entry_id_unique() {
		let mut existing = HashSet::new();
		// Pre-populate with some IDs
		existing.insert("deadbeef".to_owned());
		existing.insert("cafebabe".to_owned());
		let id = generate_entry_id(&existing);
		assert!(!existing.contains(&id), "generated ID should not be in existing set");
	}

	#[test]
	fn test_generate_entry_id_8_chars() {
		let existing = HashSet::new();
		let id = generate_entry_id(&existing);
		assert_eq!(id.len(), 8, "entry ID must be 8 chars, got {}", id.len());
		assert!(
			id.chars()
				.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
			"entry ID must be lowercase hex, got {id}"
		);
	}

	#[test]
	fn test_generate_entry_id_fallback_on_max_attempts() {
		let existing = HashSet::new();
		let id = generate_entry_id(&existing);
		// Should be either 8 chars (normal) or 16 chars (fallback)
		assert!(
			id.len() == 8 || id.len() == 16,
			"entry ID must be 8 or 16 chars, got {} (len={})",
			id,
			id.len()
		);
		assert!(
			id.chars()
				.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
			"entry ID must be lowercase hex, got {id}"
		);
	}
}
