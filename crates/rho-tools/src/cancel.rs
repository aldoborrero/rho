//! Cooperative cancellation token for long-running operations.
//!
//! Provides a lightweight [`CancelToken`] that supports deadline-based
//! timeouts and explicit abort signalling via an atomic flag.

use std::{
	sync::{
		Arc, Weak,
		atomic::{AtomicU8, Ordering},
	},
	time::{Duration, Instant},
};

use tokio::sync::Notify;

use crate::error::{Error, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Cancellation
// ─────────────────────────────────────────────────────────────────────────────

/// Reason for task abortion.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum AbortReason {
	Unknown = 1,
	Timeout = 2,
	Signal  = 3,
	User    = 4,
}

impl TryFrom<u8> for AbortReason {
	type Error = ();

	fn try_from(value: u8) -> std::result::Result<Self, ()> {
		match value {
			0 => Err(()),
			2 => Ok(Self::Timeout),
			3 => Ok(Self::Signal),
			4 => Ok(Self::User),
			_ => Ok(Self::Unknown),
		}
	}
}

#[derive(Default)]
struct Flag {
	reason:   AtomicU8,
	notifier: Notify,
}

impl Flag {
	fn cause(&self) -> Option<AbortReason> {
		self.reason.load(Ordering::Relaxed).try_into().ok()
	}

	async fn wait(&self) -> AbortReason {
		if let Some(reason) = self.cause() {
			return reason;
		}
		let notifier = self.notifier.notified();
		if let Some(reason) = self.cause() {
			return reason;
		}
		notifier.await;
		self.cause().unwrap_or(AbortReason::Unknown)
	}

	fn abort(&self, reason: AbortReason) {
		let old = self.reason.swap(reason as u8, Ordering::SeqCst);
		if old == 0 {
			self.notifier.notify_waiters();
		}
	}
}

/// Token for cooperative cancellation of blocking work.
///
/// Call `heartbeat()` periodically inside long-running work to check for
/// cancellation requests from timeouts or abort signals.
#[derive(Clone, Default)]
pub struct CancelToken {
	deadline: Option<Instant>,
	flag:     Option<Arc<Flag>>,
}

impl From<()> for CancelToken {
	fn from((): ()) -> Self {
		Self::default()
	}
}

impl CancelToken {
	/// Create a new cancel token with optional timeout.
	pub fn new(timeout_ms: Option<u32>) -> Self {
		let mut result = Self::default();
		if let Some(timeout_ms) = timeout_ms {
			result.deadline = Some(Instant::now() + Duration::from_millis(timeout_ms as u64));
		}
		// Always create a flag for abort support.
		result.flag = Some(Arc::new(Flag::default()));
		result
	}

	/// Create a cancel token with a specific timeout duration.
	pub fn with_timeout(timeout: Duration) -> Self {
		Self { deadline: Some(Instant::now() + timeout), flag: Some(Arc::new(Flag::default())) }
	}

	/// Create a cancel token with a timeout in milliseconds.
	pub fn with_timeout_ms(timeout_ms: u32) -> Self {
		Self::with_timeout(Duration::from_millis(timeout_ms as u64))
	}

	/// Check if cancellation has been requested.
	///
	/// Returns `Ok(())` if work should continue, or an error if cancelled.
	/// Call this periodically in long-running loops.
	pub fn heartbeat(&self) -> Result<()> {
		if let Some(flag) = &self.flag
			&& let Some(reason) = flag.cause()
		{
			return Err(Error::Cancelled(format!("Aborted: {reason:?}")));
		}
		if let Some(deadline) = self.deadline
			&& deadline < Instant::now()
		{
			return Err(Error::Cancelled("Aborted: Timeout".into()));
		}
		Ok(())
	}

	/// Wait for the cancel token to be aborted.
	pub async fn wait(&self) -> AbortReason {
		let flag = self.flag.as_ref();
		if let Some(flag) = flag.and_then(|f| f.cause()) {
			return flag;
		}
		let fflag = async {
			let Some(flag) = self.flag.as_ref() else {
				return std::future::pending().await;
			};
			flag.wait().await
		};

		let fttl = async {
			let Some(ttl) = self.deadline else {
				return std::future::pending().await;
			};
			tokio::time::sleep_until(ttl.into()).await;
			AbortReason::Timeout
		};

		let fuser = async {
			if tokio::signal::ctrl_c().await.is_err() {
				return std::future::pending().await;
			}
			AbortReason::User
		};

		tokio::select! {
			reason = fflag => reason,
			reason = fttl => reason,
			reason = fuser => reason,
		}
	}

	/// Request cancellation of this token.
	pub fn abort(&self) {
		if let Some(flag) = &self.flag {
			flag.abort(AbortReason::Signal);
		}
	}

	/// Get an abort token for external cancellation.
	pub fn abort_token(&self) -> AbortToken {
		AbortToken(self.flag.as_ref().map(Arc::downgrade))
	}

	/// Emplaces a cancel token if there is none, returns the abort token.
	pub fn emplace_abort_token(&mut self) -> AbortToken {
		AbortToken(Some(Arc::downgrade(self.flag.get_or_insert_default())))
	}

	/// Check if already aborted (non-blocking).
	pub fn is_aborted(&self) -> bool {
		if let Some(flag) = &self.flag
			&& flag.cause().is_some()
		{
			return true;
		}
		if let Some(deadline) = self.deadline
			&& deadline < Instant::now()
		{
			return true;
		}
		false
	}
}

/// Token for requesting cancellation from outside the task.
#[derive(Clone, Default)]
pub struct AbortToken(Option<Weak<Flag>>);

impl AbortToken {
	/// Request cancellation of the associated task.
	pub fn abort(&self, reason: AbortReason) {
		if let Some(flag) = &self.0
			&& let Some(flag) = flag.upgrade()
		{
			flag.abort(reason);
		}
	}
}
