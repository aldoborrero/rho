use std::path::{Path, PathBuf};

use anyhow::Result;
use base64::Engine;
use sha2::{Digest, Sha256};

const BLOB_PREFIX: &str = "blob:sha256:";

/// Content-addressed blob store for externalizing large binary data
/// (images) from session JSONL files.
///
/// Files are stored at `<dir>/<sha256-hex>` with no extension. The SHA-256
/// hash is computed over the raw binary data (not base64). Content-addressing
/// makes writes idempotent and provides automatic deduplication across
/// sessions.
pub struct BlobStore {
	dir: PathBuf,
}

/// Result of storing a blob.
pub struct BlobRef {
	pub hash: String,
	pub path: PathBuf,
}

impl BlobRef {
	/// Get the blob reference string: `"blob:sha256:<hash>"`.
	pub fn reference(&self) -> String {
		format!("{BLOB_PREFIX}{}", self.hash)
	}
}

impl BlobStore {
	pub const fn new(dir: PathBuf) -> Self {
		Self { dir }
	}

	/// Return the directory backing this store.
	pub fn dir(&self) -> &Path {
		&self.dir
	}

	/// Write binary data to the blob store.
	/// Returns the SHA-256 hash and file path. Idempotent.
	pub fn put(&self, data: &[u8]) -> Result<BlobRef> {
		let hash = hex_sha256(data);
		let blob_path = self.dir.join(&hash);

		// Ensure the directory exists before writing.
		std::fs::create_dir_all(&self.dir)?;
		std::fs::write(&blob_path, data)?;

		Ok(BlobRef { hash, path: blob_path })
	}

	/// Read blob by hash, returns data or `None` if not found.
	pub fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
		let blob_path = self.dir.join(hash);
		match std::fs::read(&blob_path) {
			Ok(data) => Ok(Some(data)),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
			Err(e) => Err(e.into()),
		}
	}

	/// Check if a blob exists.
	#[allow(
		clippy::unnecessary_wraps,
		reason = "consistent with `get`; future-proof for I/O errors"
	)]
	pub fn has(&self, hash: &str) -> Result<bool> {
		let blob_path = self.dir.join(hash);
		Ok(blob_path.exists())
	}
}

/// Compute the lowercase hex SHA-256 digest of `data`.
fn hex_sha256(data: &[u8]) -> String {
	let digest = Sha256::digest(data);
	// `LowerHex` formatting gives lowercase hex.
	format!("{digest:x}")
}

/// Check if a data string is a blob reference.
pub fn is_blob_ref(data: &str) -> bool {
	data.starts_with(BLOB_PREFIX)
}

/// Extract the SHA-256 hash from a blob reference string.
pub fn parse_blob_ref(data: &str) -> Option<&str> {
	data.strip_prefix(BLOB_PREFIX)
}

/// Externalize an image's base64 data to the blob store, returning a blob
/// reference. If the data is already a blob reference, returns it unchanged.
pub fn externalize_image_data(store: &BlobStore, base64_data: &str) -> Result<String> {
	if is_blob_ref(base64_data) {
		return Ok(base64_data.to_owned());
	}
	let bytes = base64::engine::general_purpose::STANDARD.decode(base64_data)?;
	let blob_ref = store.put(&bytes)?;
	Ok(blob_ref.reference())
}

/// Resolve a blob reference back to base64 data.
/// If the data is not a blob reference, returns it unchanged.
/// If the blob is missing, returns the ref as-is (graceful degradation).
pub fn resolve_image_data(store: &BlobStore, data: &str) -> Result<String> {
	let Some(hash) = parse_blob_ref(data) else {
		return Ok(data.to_owned());
	};
	match store.get(hash)? {
		Some(bytes) => Ok(base64::engine::general_purpose::STANDARD.encode(bytes)),
		None => {
			// Graceful degradation: return ref as-is.
			Ok(data.to_owned())
		},
	}
}

#[cfg(test)]
mod tests {
	use tempfile::TempDir;

	use super::*;

	fn make_store() -> (TempDir, BlobStore) {
		let tmp = TempDir::new().unwrap();
		let store = BlobStore::new(tmp.path().join("blobs"));
		(tmp, store)
	}

	#[test]
	fn test_put_and_get() {
		let (_tmp, store) = make_store();
		let data = b"hello world";
		let blob_ref = store.put(data).unwrap();

		// Hash should be non-empty hex.
		assert!(!blob_ref.hash.is_empty());
		assert!(blob_ref.hash.chars().all(|c| c.is_ascii_hexdigit()));

		// Retrieve and verify contents.
		let retrieved = store
			.get(&blob_ref.hash)
			.unwrap()
			.expect("blob should exist");
		assert_eq!(retrieved, data);
	}

	#[test]
	fn test_put_is_idempotent() {
		let (_tmp, store) = make_store();
		let data = b"idempotent data";

		let ref1 = store.put(data).unwrap();
		let ref2 = store.put(data).unwrap();

		assert_eq!(ref1.hash, ref2.hash);
		assert_eq!(ref1.path, ref2.path);

		// File should still contain the original data.
		let retrieved = store.get(&ref1.hash).unwrap().expect("blob should exist");
		assert_eq!(retrieved, data);
	}

	#[test]
	fn test_get_missing() {
		let (_tmp, store) = make_store();
		let result = store
			.get("0000000000000000000000000000000000000000000000000000000000000000")
			.unwrap();
		assert!(result.is_none());
	}

	#[test]
	fn test_has_exists() {
		let (_tmp, store) = make_store();
		let blob_ref = store.put(b"exists").unwrap();
		assert!(store.has(&blob_ref.hash).unwrap());
	}

	#[test]
	fn test_has_missing() {
		let (_tmp, store) = make_store();
		assert!(
			!store
				.has("0000000000000000000000000000000000000000000000000000000000000000")
				.unwrap()
		);
	}

	#[test]
	fn test_is_blob_ref() {
		assert!(is_blob_ref("blob:sha256:abc123"));
		assert!(!is_blob_ref("other"));
		assert!(!is_blob_ref(""));
		assert!(!is_blob_ref("blob:sha256"));
	}

	#[test]
	fn test_parse_blob_ref() {
		assert_eq!(parse_blob_ref("blob:sha256:abc123"), Some("abc123"));
		assert_eq!(parse_blob_ref("blob:sha256:deadbeef0123456789"), Some("deadbeef0123456789"));
		assert_eq!(parse_blob_ref("other"), None);
		assert_eq!(parse_blob_ref(""), None);
	}

	#[test]
	fn test_externalize_and_resolve_roundtrip() {
		let (_tmp, store) = make_store();

		// Encode some raw bytes as base64.
		let raw = b"image pixel data here";
		let original_b64 = base64::engine::general_purpose::STANDARD.encode(raw);

		// Externalize: base64 -> blob reference.
		let blob_ref_str = externalize_image_data(&store, &original_b64).unwrap();
		assert!(is_blob_ref(&blob_ref_str), "should be a blob reference");

		// Externalizing again should be a no-op (returns the same ref).
		let again = externalize_image_data(&store, &blob_ref_str).unwrap();
		assert_eq!(again, blob_ref_str);

		// Resolve: blob reference -> base64.
		let resolved_b64 = resolve_image_data(&store, &blob_ref_str).unwrap();
		assert_eq!(resolved_b64, original_b64);

		// Resolving non-ref data returns it unchanged.
		let plain = "not a ref";
		assert_eq!(resolve_image_data(&store, plain).unwrap(), plain);

		// Resolving a ref to a missing blob returns the ref as-is (graceful
		// degradation).
		let missing_ref =
			"blob:sha256:0000000000000000000000000000000000000000000000000000000000000000";
		assert_eq!(resolve_image_data(&store, missing_ref).unwrap(), missing_ref);
	}
}
