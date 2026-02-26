//! Trait-based storage abstraction for session persistence.
//!
//! Defines [`SessionStorage`] and [`SessionWriter`] traits that abstract over
//! filesystem operations, enabling both real filesystem and in-memory
//! implementations. The in-memory implementation ([`MemorySessionStorage`]) is
//! provided here for testing purposes.

use std::{
	collections::HashMap,
	fs::{self, File, OpenOptions},
	io::Write,
	path::{Path, PathBuf},
	sync::{Arc, Mutex},
	time::SystemTime,
};

use anyhow::{Result, bail};

// ---------------------------------------------------------------------------
// FileStat
// ---------------------------------------------------------------------------

/// Filesystem stat info.
#[derive(Debug, Clone)]
pub struct FileStat {
	pub size:     u64,
	pub modified: SystemTime,
}

// ---------------------------------------------------------------------------
// SessionWriter trait
// ---------------------------------------------------------------------------

/// Writer for appending lines to a session file.
pub trait SessionWriter: Send {
	/// Write a line (the implementation must append a newline).
	fn write_line(&mut self, line: &str) -> Result<()>;
	/// Flush buffered data to the underlying storage.
	fn flush(&mut self) -> Result<()>;
	/// Close the writer (flush + release resources).
	fn close(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// SessionStorage trait
// ---------------------------------------------------------------------------

/// Abstraction over filesystem operations for session persistence.
///
/// Enables testing with in-memory storage instead of hitting the real
/// filesystem.
pub trait SessionStorage: Send {
	/// Create a directory and all parent directories.
	fn ensure_dir(&self, dir: &Path) -> Result<()>;
	/// Check if a path exists.
	fn exists(&self, path: &Path) -> bool;
	/// Read the entire contents of a file as a string.
	fn read_text(&self, path: &Path) -> Result<String>;
	/// Read the first `max_bytes` of a file as a string.
	///
	/// If the file is shorter than `max_bytes`, the full content is returned.
	/// The result is truncated at a valid UTF-8 boundary.
	fn read_text_prefix(&self, path: &Path, max_bytes: usize) -> Result<String>;
	/// Write a string to a file (overwriting if it exists). Creates parent dirs.
	fn write_text(&self, path: &Path, content: &str) -> Result<()>;
	/// Get file metadata (size and modified time).
	fn stat(&self, path: &Path) -> Result<FileStat>;
	/// List files in a directory matching a given extension (e.g., ".jsonl").
	fn list_files(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>>;
	/// List immediate subdirectories of a directory.
	fn list_dirs(&self, dir: &Path) -> Result<Vec<PathBuf>>;
	/// Rename a file.
	fn rename(&self, from: &Path, to: &Path) -> Result<()>;
	/// Remove a file.
	fn remove(&self, path: &Path) -> Result<()>;
	/// Open a writer for appending or overwriting.
	fn open_writer(&self, path: &Path, append: bool) -> Result<Box<dyn SessionWriter>>;
}

// ---------------------------------------------------------------------------
// MemorySessionStorage
// ---------------------------------------------------------------------------

/// Internal data for a single in-memory file.
#[derive(Debug, Clone)]
struct FileData {
	content:  String,
	modified: SystemTime,
}

/// In-memory implementation of [`SessionStorage`] for testing.
///
/// Uses an `Arc<Mutex<HashMap>>` so that writers can share access to the
/// backing store while satisfying the `Send` requirement.
pub struct MemorySessionStorage {
	files: Arc<Mutex<HashMap<PathBuf, FileData>>>,
}

impl MemorySessionStorage {
	/// Create a new empty in-memory storage.
	pub fn new() -> Self {
		Self { files: Arc::new(Mutex::new(HashMap::new())) }
	}
}

impl Default for MemorySessionStorage {
	fn default() -> Self {
		Self::new()
	}
}

impl SessionStorage for MemorySessionStorage {
	fn ensure_dir(&self, _dir: &Path) -> Result<()> {
		// No-op: memory storage doesn't need directories.
		Ok(())
	}

	fn exists(&self, path: &Path) -> bool {
		let files = self.files.lock().expect("lock poisoned");
		files.contains_key(path)
	}

	fn read_text(&self, path: &Path) -> Result<String> {
		let files = self.files.lock().expect("lock poisoned");
		match files.get(path) {
			Some(data) => Ok(data.content.clone()),
			None => bail!("file not found: {}", path.display()),
		}
	}

	fn read_text_prefix(&self, path: &Path, max_bytes: usize) -> Result<String> {
		let files = self.files.lock().expect("lock poisoned");
		match files.get(path) {
			Some(data) => {
				if data.content.len() <= max_bytes {
					Ok(data.content.clone())
				} else {
					// Truncate at a valid UTF-8 char boundary.
					let bytes = data.content.as_bytes();
					let mut end = max_bytes;
					while end > 0 && !data.content.is_char_boundary(end) {
						end -= 1;
					}
					Ok(String::from_utf8(bytes[..end].to_vec()).expect("sliced at char boundary"))
				}
			},
			None => bail!("file not found: {}", path.display()),
		}
	}

	fn write_text(&self, path: &Path, content: &str) -> Result<()> {
		let mut files = self.files.lock().expect("lock poisoned");
		files.insert(path.to_path_buf(), FileData {
			content:  content.to_owned(),
			modified: SystemTime::now(),
		});
		Ok(())
	}

	fn stat(&self, path: &Path) -> Result<FileStat> {
		let files = self.files.lock().expect("lock poisoned");
		match files.get(path) {
			Some(data) => {
				Ok(FileStat { size: data.content.len() as u64, modified: data.modified })
			},
			None => bail!("file not found: {}", path.display()),
		}
	}

	fn list_files(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>> {
		let files = self.files.lock().expect("lock poisoned");
		let mut result: Vec<PathBuf> = files
			.keys()
			.filter(|p| {
				p.parent() == Some(dir)
					&& p
						.extension()
						.and_then(|e| e.to_str())
						.is_some_and(|e| format!(".{e}") == extension)
			})
			.cloned()
			.collect();
		result.sort();
		Ok(result)
	}

	fn list_dirs(&self, dir: &Path) -> Result<Vec<PathBuf>> {
		let files = self.files.lock().expect("lock poisoned");
		let mut dirs: Vec<PathBuf> = files
			.keys()
			.filter_map(|p| {
				// If the file's grandparent (or ancestor) is `dir`, collect
				// the immediate child directory.
				let parent = p.parent()?;
				if parent == dir {
					// File is directly in `dir`, not in a subdirectory.
					return None;
				}
				// Walk up until we find a component whose parent is `dir`.
				let mut current = parent;
				loop {
					let cp = current.parent()?;
					if cp == dir {
						return Some(current.to_path_buf());
					}
					current = cp;
				}
			})
			.collect::<std::collections::HashSet<_>>()
			.into_iter()
			.collect();
		dirs.sort();
		Ok(dirs)
	}

	fn rename(&self, from: &Path, to: &Path) -> Result<()> {
		let mut files = self.files.lock().expect("lock poisoned");
		match files.remove(from) {
			Some(data) => {
				files.insert(to.to_path_buf(), data);
				Ok(())
			},
			None => bail!("file not found: {}", from.display()),
		}
	}

	fn remove(&self, path: &Path) -> Result<()> {
		let mut files = self.files.lock().expect("lock poisoned");
		files.remove(path);
		Ok(())
	}

	fn open_writer(&self, path: &Path, append: bool) -> Result<Box<dyn SessionWriter>> {
		let mut files = self.files.lock().expect("lock poisoned");
		if append {
			// Ensure the file entry exists for append mode.
			files
				.entry(path.to_path_buf())
				.or_insert_with(|| FileData { content: String::new(), modified: SystemTime::now() });
		} else {
			// Overwrite: start with empty content.
			files.insert(path.to_path_buf(), FileData {
				content:  String::new(),
				modified: SystemTime::now(),
			});
		}
		drop(files);

		Ok(Box::new(MemorySessionWriter {
			path:   path.to_path_buf(),
			files:  Arc::clone(&self.files),
			buffer: String::new(),
		}))
	}
}

// ---------------------------------------------------------------------------
// MemorySessionWriter
// ---------------------------------------------------------------------------

/// In-memory writer that buffers lines and flushes to the shared file map.
struct MemorySessionWriter {
	path:   PathBuf,
	files:  Arc<Mutex<HashMap<PathBuf, FileData>>>,
	buffer: String,
}

impl SessionWriter for MemorySessionWriter {
	fn write_line(&mut self, line: &str) -> Result<()> {
		self.buffer.push_str(line);
		self.buffer.push('\n');
		Ok(())
	}

	fn flush(&mut self) -> Result<()> {
		if !self.buffer.is_empty() {
			let mut files = self.files.lock().expect("lock poisoned");
			let entry = files
				.entry(self.path.clone())
				.or_insert_with(|| FileData { content: String::new(), modified: SystemTime::now() });
			entry.content.push_str(&self.buffer);
			entry.modified = SystemTime::now();
			self.buffer.clear();
		}
		Ok(())
	}

	fn close(&mut self) -> Result<()> {
		self.flush()
	}
}

// ---------------------------------------------------------------------------
// FileSessionStorage
// ---------------------------------------------------------------------------

/// File-system-backed session storage.
pub struct FileSessionStorage;

impl SessionStorage for FileSessionStorage {
	fn ensure_dir(&self, dir: &Path) -> Result<()> {
		fs::create_dir_all(dir)?;
		Ok(())
	}

	fn exists(&self, path: &Path) -> bool {
		path.exists()
	}

	fn read_text(&self, path: &Path) -> Result<String> {
		Ok(fs::read_to_string(path)?)
	}

	fn read_text_prefix(&self, path: &Path, max_bytes: usize) -> Result<String> {
		use std::io::Read;
		let mut file = File::open(path)?;
		let mut buf = vec![0u8; max_bytes];
		let n = file.read(&mut buf)?;
		buf.truncate(n);
		// Truncate at the last valid UTF-8 char boundary instead of lossy conversion.
		let mut end = n;
		while end > 0 && std::str::from_utf8(&buf[..end]).is_err() {
			end -= 1;
		}
		Ok(String::from_utf8(buf[..end].to_vec()).expect("truncated at valid UTF-8 boundary"))
	}

	fn write_text(&self, path: &Path, content: &str) -> Result<()> {
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(path, content)?;
		Ok(())
	}

	fn stat(&self, path: &Path) -> Result<FileStat> {
		let meta = fs::metadata(path)?;
		Ok(FileStat { size: meta.len(), modified: meta.modified()? })
	}

	fn list_files(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>> {
		let mut files = Vec::new();
		if !dir.exists() {
			return Ok(files);
		}
		for entry in fs::read_dir(dir)? {
			let entry = entry?;
			let path = entry.path();
			if path.is_file()
				&& let Some(ext) = path.extension()
			{
				// extension param is like ".jsonl" — compare without the dot
				let ext_no_dot = extension.strip_prefix('.').unwrap_or(extension);
				if ext == ext_no_dot {
					files.push(path);
				}
			}
		}
		Ok(files)
	}

	fn list_dirs(&self, dir: &Path) -> Result<Vec<PathBuf>> {
		let mut dirs = Vec::new();
		if !dir.exists() {
			return Ok(dirs);
		}
		for entry in fs::read_dir(dir)? {
			let entry = entry?;
			let path = entry.path();
			if path.is_dir() {
				dirs.push(path);
			}
		}
		Ok(dirs)
	}

	fn rename(&self, from: &Path, to: &Path) -> Result<()> {
		fs::rename(from, to)?;
		Ok(())
	}

	fn remove(&self, path: &Path) -> Result<()> {
		fs::remove_file(path)?;
		Ok(())
	}

	fn open_writer(&self, path: &Path, append: bool) -> Result<Box<dyn SessionWriter>> {
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		let file = OpenOptions::new()
			.create(true)
			.write(true)
			.append(append)
			.truncate(!append)
			.open(path)?;
		Ok(Box::new(FileSessionWriter { file }))
	}
}

// ---------------------------------------------------------------------------
// FileSessionWriter
// ---------------------------------------------------------------------------

/// Writer backed by a real file.
///
/// Uses a single `write_all` call per entry (line + newline combined) to
/// avoid partial writes that could leave the session file unrecoverable
/// after a crash.
struct FileSessionWriter {
	file: File,
}

impl SessionWriter for FileSessionWriter {
	fn write_line(&mut self, line: &str) -> Result<()> {
		let mut buf = Vec::with_capacity(line.len() + 1);
		buf.extend_from_slice(line.as_bytes());
		buf.push(b'\n');
		self.file.write_all(&buf)?;
		Ok(())
	}

	fn flush(&mut self) -> Result<()> {
		self.file.sync_all()?;
		Ok(())
	}

	fn close(&mut self) -> Result<()> {
		self.flush()
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use std::path::Path;

	use super::*;

	fn storage() -> MemorySessionStorage {
		MemorySessionStorage::new()
	}

	#[test]
	fn test_write_and_read() {
		let s = storage();
		let p = Path::new("/tmp/test.txt");
		s.write_text(p, "hello world").unwrap();
		assert_eq!(s.read_text(p).unwrap(), "hello world");
	}

	#[test]
	fn test_exists_after_write() {
		let s = storage();
		let p = Path::new("/tmp/test.txt");
		assert!(!s.exists(p));
		s.write_text(p, "data").unwrap();
		assert!(s.exists(p));
	}

	#[test]
	fn test_read_text_prefix() {
		let s = storage();
		let p = Path::new("/tmp/test.txt");
		s.write_text(p, "hello world").unwrap();

		// Read only first 5 bytes.
		let prefix = s.read_text_prefix(p, 5).unwrap();
		assert_eq!(prefix, "hello");

		// Reading more than the file size returns full content.
		let full = s.read_text_prefix(p, 1000).unwrap();
		assert_eq!(full, "hello world");

		// Test UTF-8 boundary: write a multi-byte character and truncate in the middle.
		// The char U+00E9 ('e' with acute) is 2 bytes in UTF-8.
		s.write_text(p, "caf\u{00e9}!").unwrap();
		// "caf" = 3 bytes, then e-acute = 2 bytes (bytes 3..5), then "!" = 1 byte
		// Truncating at 4 bytes would land in the middle of e-acute, so it should
		// back up to byte 3 and return "caf".
		let prefix = s.read_text_prefix(p, 4).unwrap();
		assert_eq!(prefix, "caf");
	}

	#[test]
	fn test_list_files_by_extension() {
		let s = storage();
		let dir = Path::new("/sessions");
		s.write_text(&dir.join("a.jsonl"), "line1").unwrap();
		s.write_text(&dir.join("b.jsonl"), "line2").unwrap();
		s.write_text(&dir.join("c.txt"), "other").unwrap();

		let jsonl_files = s.list_files(dir, ".jsonl").unwrap();
		assert_eq!(jsonl_files.len(), 2);
		assert!(
			jsonl_files
				.iter()
				.all(|p| p.extension().is_some_and(|e| e == "jsonl"))
		);

		// No .md files expected.
		let md_files = s.list_files(dir, ".md").unwrap();
		assert!(md_files.is_empty());
	}

	#[test]
	fn test_rename() {
		let s = storage();
		let old = Path::new("/tmp/old.txt");
		let new = Path::new("/tmp/new.txt");
		s.write_text(old, "content").unwrap();

		s.rename(old, new).unwrap();
		assert!(!s.exists(old));
		assert!(s.exists(new));
		assert_eq!(s.read_text(new).unwrap(), "content");
	}

	#[test]
	fn test_remove() {
		let s = storage();
		let p = Path::new("/tmp/test.txt");
		s.write_text(p, "data").unwrap();
		assert!(s.exists(p));

		s.remove(p).unwrap();
		assert!(!s.exists(p));
	}

	#[test]
	fn test_writer_append() {
		let s = storage();
		let p = Path::new("/tmp/session.jsonl");

		let mut writer = s.open_writer(p, false).unwrap();
		writer.write_line("line1").unwrap();
		writer.write_line("line2").unwrap();
		writer.flush().unwrap();

		// Open another writer in append mode.
		let mut writer2 = s.open_writer(p, true).unwrap();
		writer2.write_line("line3").unwrap();
		writer2.close().unwrap();

		let content = s.read_text(p).unwrap();
		assert_eq!(content, "line1\nline2\nline3\n");
	}

	#[test]
	fn test_stat_size() {
		let s = storage();
		let p = Path::new("/tmp/test.txt");
		let content = "hello world";
		s.write_text(p, content).unwrap();

		let stat = s.stat(p).unwrap();
		assert_eq!(stat.size, content.len() as u64);
	}

	// -----------------------------------------------------------------------
	// FileSessionStorage tests
	// -----------------------------------------------------------------------

	fn file_storage() -> (tempfile::TempDir, FileSessionStorage) {
		(tempfile::tempdir().unwrap(), FileSessionStorage)
	}

	#[test]
	fn test_file_storage_roundtrip() {
		let (tmp, s) = file_storage();
		let p = tmp.path().join("hello.txt");
		let content = "hello world from file storage";
		s.write_text(&p, content).unwrap();
		assert_eq!(s.read_text(&p).unwrap(), content);
	}

	#[test]
	fn test_file_storage_list_jsonl() {
		let (tmp, s) = file_storage();
		let dir = tmp.path().join("sessions");
		s.ensure_dir(&dir).unwrap();

		s.write_text(&dir.join("a.jsonl"), "line1").unwrap();
		s.write_text(&dir.join("b.jsonl"), "line2").unwrap();
		s.write_text(&dir.join("c.txt"), "other").unwrap();

		let mut jsonl_files = s.list_files(&dir, ".jsonl").unwrap();
		jsonl_files.sort();
		assert_eq!(jsonl_files.len(), 2);
		assert!(
			jsonl_files
				.iter()
				.all(|p| p.extension().is_some_and(|e| e == "jsonl"))
		);

		// No .md files expected.
		let md_files = s.list_files(&dir, ".md").unwrap();
		assert!(md_files.is_empty());
	}

	#[test]
	fn test_file_writer_append() {
		let (tmp, s) = file_storage();
		let p = tmp.path().join("append.log");

		let mut writer = s.open_writer(&p, false).unwrap();
		writer.write_line("line1").unwrap();
		writer.write_line("line2").unwrap();
		writer.write_line("line3").unwrap();
		writer.close().unwrap();

		let content = s.read_text(&p).unwrap();
		let lines: Vec<&str> = content.lines().collect();
		assert_eq!(lines, vec!["line1", "line2", "line3"]);
	}

	#[test]
	fn test_file_storage_rename() {
		let (tmp, s) = file_storage();
		let old = tmp.path().join("old.txt");
		let new = tmp.path().join("new.txt");

		s.write_text(&old, "rename me").unwrap();
		assert!(s.exists(&old));

		s.rename(&old, &new).unwrap();
		assert!(!s.exists(&old));
		assert!(s.exists(&new));
		assert_eq!(s.read_text(&new).unwrap(), "rename me");
	}

	#[test]
	fn test_file_storage_ensure_dir_nested() {
		let (tmp, s) = file_storage();
		let nested = tmp.path().join("a").join("b").join("c").join("d");

		s.ensure_dir(&nested).unwrap();
		assert!(nested.exists());
		assert!(nested.is_dir());
	}
}
