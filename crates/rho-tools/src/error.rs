//! Unified error types for pi-tools.

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("Cancelled: {0}")]
	Cancelled(String),
	#[error("Invalid pattern: {0}")]
	InvalidPattern(String),
	#[error("Path error: {0}")]
	PathError(String),
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
	#[error("{0}")]
	Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
	pub fn from_reason(reason: impl Into<String>) -> Self {
		Self::Other(reason.into())
	}
}
