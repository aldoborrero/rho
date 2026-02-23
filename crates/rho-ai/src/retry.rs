use std::sync::LazyLock;

use regex::Regex;

// ---------------------------------------------------------------------------
// RetryConfig
// ---------------------------------------------------------------------------

/// Configuration for automatic retry with exponential backoff.
#[derive(Debug, Clone)]
pub struct RetryConfig {
	/// Whether retries are enabled at all.
	pub enabled:       bool,
	/// Maximum number of retry attempts.
	pub max_retries:   u32,
	/// Base delay in milliseconds (doubles each attempt).
	pub base_delay_ms: u64,
	/// Maximum delay cap in milliseconds.
	pub max_delay_ms:  u64,
}

impl Default for RetryConfig {
	fn default() -> Self {
		Self { enabled: true, max_retries: 3, base_delay_ms: 1000, max_delay_ms: 30000 }
	}
}

// ---------------------------------------------------------------------------
// Compiled regex patterns (LazyLock)
// ---------------------------------------------------------------------------

/// Regex pattern matching transient/retryable error messages.
///
/// Matches overloaded, rate limit, usage limit, too many requests,
/// service unavailable, server error, internal error, connection error/reset,
/// fetch failed, ECONNRESET, ETIMEDOUT, timeout, timed out, socket hang up,
/// and capacity.
static RETRYABLE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
        r"(?i)overloaded|rate.?limit|usage.?limit|too many requests|service.?unavailable|server error|internal error|connection.?error|connection.?reset|fetch failed|ECONNRESET|ETIMEDOUT|timeout|timed out|socket hang up|capacity",
    )
    .expect("RETRYABLE_PATTERN regex is valid")
});

/// Regex for parsing `retry-after-ms` values from error messages.
static RETRY_AFTER_MS_RE: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?i)retry-after-ms\s*[:=]\s*(\d+)").expect("RETRY_AFTER_MS_RE regex is valid")
});

/// Regex for parsing `retry-after` values (in seconds) from error messages.
static RETRY_AFTER_RE: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(?i)retry-after\s*[:=]\s*(\d+)").expect("RETRY_AFTER_RE regex is valid")
});

// ---------------------------------------------------------------------------
// is_retryable
// ---------------------------------------------------------------------------

/// Check whether an HTTP status code and/or error message indicates a
/// transient, retryable condition.
///
/// Status-code logic:
/// - 408, 429 -> retryable (request timeout, rate limit)
/// - 500-599  -> retryable (server errors)
/// - other 4xx -> **not** retryable (client errors)
///
/// If no status code matches (or none is provided), the error message is
/// tested against [`RETRYABLE_PATTERN`] (case-insensitive).
pub fn is_retryable(status: Option<u16>, message: &str) -> bool {
	if let Some(code) = status {
		if code >= 500 {
			return true;
		}
		if code == 408 || code == 429 {
			return true;
		}
		// Other 4xx codes are definitively not retryable.
		if code >= 400 && code < 500 {
			return false;
		}
	}

	RETRYABLE_PATTERN.is_match(message)
}

// ---------------------------------------------------------------------------
// calculate_backoff
// ---------------------------------------------------------------------------

/// Calculate exponential backoff delay for a given retry attempt.
///
/// Formula: `min(base_delay_ms * 2^(attempt - 1), max_delay_ms)`
///
/// If `retry_after_ms` is provided (e.g. parsed from a `Retry-After` header),
/// the returned delay is at least that value.
pub fn calculate_backoff(config: &RetryConfig, attempt: u32, retry_after_ms: Option<u64>) -> u64 {
	let exponential = config
		.base_delay_ms
		.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
	let capped = exponential.min(config.max_delay_ms);

	match retry_after_ms {
		Some(ra) => capped.max(ra),
		None => capped,
	}
}

// ---------------------------------------------------------------------------
// parse_retry_after_from_error
// ---------------------------------------------------------------------------

/// Parse a retry-after delay (in milliseconds) from an error message string.
///
/// Supports two formats embedded in the message:
/// - `retry-after-ms=5000` (or `: 5000`) -> returns `Some(5000)`
/// - `retry-after=3` (or `: 3`)          -> returns `Some(3000)` (seconds ->
///   ms)
///
/// Returns `None` if no retry-after value is found.
pub fn parse_retry_after_from_error(message: &str) -> Option<u64> {
	// Check retry-after-ms first (already in milliseconds)
	if let Some(caps) = RETRY_AFTER_MS_RE.captures(message) {
		return caps[1].parse().ok();
	}
	// Fall back to retry-after (in seconds, convert to ms)
	if let Some(caps) = RETRY_AFTER_RE.captures(message) {
		return caps[1].parse::<u64>().ok().map(|s| s * 1000);
	}
	None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	// --- is_retryable ---

	#[test]
	fn retryable_server_errors() {
		assert!(is_retryable(Some(500), ""));
		assert!(is_retryable(Some(502), ""));
		assert!(is_retryable(Some(503), ""));
		assert!(is_retryable(Some(529), ""));
	}

	#[test]
	fn retryable_rate_limit() {
		assert!(is_retryable(Some(429), ""));
		assert!(is_retryable(Some(408), ""));
	}

	#[test]
	fn retryable_message_patterns() {
		assert!(is_retryable(None, "server is overloaded"));
		assert!(is_retryable(None, "rate limit exceeded"));
		assert!(is_retryable(None, "service unavailable"));
		assert!(is_retryable(None, "connection reset"));
		assert!(is_retryable(None, "ECONNRESET"));
		assert!(is_retryable(None, "fetch failed"));
		assert!(is_retryable(None, "too many requests"));
	}

	#[test]
	fn not_retryable_client_errors() {
		assert!(!is_retryable(Some(400), "bad request"));
		assert!(!is_retryable(Some(401), "unauthorized"));
		assert!(!is_retryable(Some(403), "forbidden"));
		assert!(!is_retryable(Some(404), "not found"));
	}

	#[test]
	fn retryable_case_insensitive() {
		assert!(is_retryable(None, "OVERLOADED"));
		assert!(is_retryable(None, "Rate Limit"));
	}

	// --- calculate_backoff ---

	#[test]
	fn backoff_exponential() {
		let config = RetryConfig {
			enabled:       true,
			max_retries:   5,
			base_delay_ms: 1000,
			max_delay_ms:  30000,
		};
		assert_eq!(calculate_backoff(&config, 1, None), 1000);
		assert_eq!(calculate_backoff(&config, 2, None), 2000);
		assert_eq!(calculate_backoff(&config, 3, None), 4000);
	}

	#[test]
	fn backoff_capped_at_max() {
		let config = RetryConfig {
			enabled:       true,
			max_retries:   10,
			base_delay_ms: 1000,
			max_delay_ms:  5000,
		};
		assert_eq!(calculate_backoff(&config, 10, None), 5000);
	}

	#[test]
	fn backoff_respects_retry_after() {
		let config = RetryConfig::default();
		assert_eq!(calculate_backoff(&config, 1, Some(10000)), 10000);
	}

	// --- parse_retry_after_from_error ---

	#[test]
	fn parse_retry_after_ms_from_message() {
		assert_eq!(parse_retry_after_from_error("failed retry-after-ms=5000"), Some(5000));
	}

	#[test]
	fn parse_retry_after_seconds_from_message() {
		assert_eq!(parse_retry_after_from_error("failed retry-after=3"), Some(3000));
	}

	#[test]
	fn parse_no_retry_after() {
		assert_eq!(parse_retry_after_from_error("some error"), None);
	}

	// --- RetryConfig default ---

	#[test]
	fn default_config() {
		let config = RetryConfig::default();
		assert!(config.enabled);
		assert_eq!(config.max_retries, 3);
		assert_eq!(config.base_delay_ms, 1000);
		assert_eq!(config.max_delay_ms, 30000);
	}
}
