//! Session management with tree-structured entries and JSONL persistence.
//!
//! The [`SessionManager`] is the central coordinator for session state. It
//! maintains an append-only log of [`SessionEntry`] values linked into a tree
//! via parent pointers. Persistence is handled through the [`SessionStorage`]
//! and [`SessionWriter`] traits, enabling both filesystem and in-memory
//! backends.

pub mod blob_store;
pub mod breadcrumb;
pub mod context;
pub mod paths;
pub mod snowflake;
pub mod storage;
pub mod types;

use std::{
	collections::{HashMap, HashSet},
	path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use chrono::Utc;
use serde_json::Value;

use self::{
	blob_store::BlobStore,
	paths::session_file_name,
	storage::{FileSessionStorage, MemorySessionStorage, SessionStorage, SessionWriter},
	types::{
		BranchSummaryEntry, CompactionEntry, CustomEntry, FileEntry, ForkResult, ModeChangeEntry,
		ModelChangeEntry, SessionEntry, SessionHeader, SessionInfo, SessionMessageEntry,
		ThinkingLevelChangeEntry,
	},
};
use crate::{
	ai::types::Message,
	config::{get_blobs_dir, get_default_agent_dir, get_default_session_dir},
};

// ---------------------------------------------------------------------------
// SessionManager
// ---------------------------------------------------------------------------

/// Tree-structured session manager with JSONL persistence.
///
/// Entries are stored in file order (append-only) and linked into a tree
/// via `parent_id` pointers. The `leaf_id` tracks the current conversation
/// position (tip of the active branch).
pub struct SessionManager {
	header:            SessionHeader,
	entries:           Vec<SessionEntry>,
	by_id:             HashMap<String, usize>,
	#[allow(dead_code, reason = "scaffolding for Task 12: fork/branching with labels")]
	labels_by_id:      HashMap<String, String>,
	leaf_id:           Option<String>,
	entry_ids:         HashSet<String>,
	storage:           Box<dyn SessionStorage>,
	session_file:      Option<PathBuf>,
	blob_store:        Option<BlobStore>,
	writer:            Option<Box<dyn SessionWriter>>,
	is_dirty:          bool,
	/// Cached flat message list extracted from entries (for the old API).
	messages_cache:    Vec<Message>,
	/// Whether a message entry has been seen (for delayed persistence).
	has_message_entry: bool,
	/// Session directory (for file-based sessions).
	session_dir:       Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Factory methods
// ---------------------------------------------------------------------------

impl SessionManager {
	/// Create a new session rooted at `cwd`.
	///
	/// The session directory defaults to `~/.rho/agent/sessions/<encoded-cwd>/`
	/// unless overridden by `session_dir`.
	///
	/// **Delayed persistence:** The JSONL file is NOT created here. It is
	/// created when the first entry is appended AND a message entry has been
	/// seen.
	#[allow(
		clippy::unnecessary_wraps,
		reason = "future implementations may fail; keeping Result for API consistency"
	)]
	pub fn create(cwd: &Path, session_dir: Option<&Path>) -> Result<Self> {
		let session_id = snowflake::next();
		let dir = match session_dir {
			Some(p) => p.to_path_buf(),
			None => get_default_session_dir(cwd),
		};

		let header = SessionHeader {
			r#type:         "session".to_owned(),
			version:        1,
			id:             session_id,
			title:          None,
			timestamp:      Utc::now().to_rfc3339(),
			cwd:            cwd.to_string_lossy().to_string(),
			parent_session: None,
		};

		let file_name = session_file_name(&header.id);
		let file_path = dir.join(&file_name);

		let storage: Box<dyn SessionStorage> = Box::new(storage::FileSessionStorage);

		Ok(Self {
			header,
			entries: Vec::new(),
			by_id: HashMap::new(),
			labels_by_id: HashMap::new(),
			leaf_id: None,
			entry_ids: HashSet::new(),
			storage,
			session_file: Some(file_path),
			blob_store: Some(BlobStore::new(get_blobs_dir())),
			writer: None,
			is_dirty: false,
			messages_cache: Vec::new(),
			has_message_entry: false,
			session_dir: Some(dir),
		})
	}

	/// Open an existing session from a JSONL file.
	///
	/// Reads the file, parses the header and entries, rebuilds the index,
	/// and determines the current leaf (last entry in the longest chain from
	/// root).
	pub fn open(file_path: &Path, session_dir: Option<&Path>) -> Result<Self> {
		let storage: Box<dyn SessionStorage> = Box::new(storage::FileSessionStorage);

		let content = storage.read_text(file_path)?;
		let (header, entries) = Self::parse_jsonl(&content)?;

		let dir = session_dir
			.map(|p| p.to_path_buf())
			.or_else(|| file_path.parent().map(|p| p.to_path_buf()));

		let mut mgr = Self {
			header,
			entries: Vec::new(),
			by_id: HashMap::new(),
			labels_by_id: HashMap::new(),
			leaf_id: None,
			entry_ids: HashSet::new(),
			storage,
			session_file: Some(file_path.to_path_buf()),
			blob_store: Some(BlobStore::new(get_blobs_dir())),
			writer: None,
			is_dirty: false,
			messages_cache: Vec::new(),
			has_message_entry: false,
			session_dir: dir,
		};

		// Index all entries.
		for entry in entries {
			let _ = mgr.index_entry(entry);
		}

		// Find the leaf: the last entry in the chain from root.
		mgr.leaf_id = mgr.find_leaf();

		// Rebuild the messages cache.
		mgr.rebuild_messages_cache();

		// We've seen message entries if any exist.
		mgr.has_message_entry = mgr
			.entries
			.iter()
			.any(|e| matches!(e, SessionEntry::Message(_)));

		Ok(mgr)
	}

	/// Create an in-memory session with no persistence.
	pub fn in_memory() -> Self {
		let session_id = snowflake::next();
		let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

		let header = SessionHeader {
			r#type:         "session".to_owned(),
			version:        1,
			id:             session_id,
			title:          None,
			timestamp:      Utc::now().to_rfc3339(),
			cwd:            cwd.to_string_lossy().to_string(),
			parent_session: None,
		};

		Self {
			header,
			entries: Vec::new(),
			by_id: HashMap::new(),
			labels_by_id: HashMap::new(),
			leaf_id: None,
			entry_ids: HashSet::new(),
			storage: Box::new(MemorySessionStorage::new()),
			session_file: None,
			blob_store: None,
			writer: None,
			is_dirty: false,
			messages_cache: Vec::new(),
			has_message_entry: false,
			session_dir: None,
		}
	}

	/// Continue the most recent session for `cwd`.
	///
	/// First checks the terminal breadcrumb; if none exists, falls back to
	/// scanning the session directory for the most recently modified JSONL
	/// file.
	pub fn continue_recent_in(cwd: &Path, session_dir: Option<&Path>) -> Result<Self> {
		// Try breadcrumb first.
		if let Some(file_path) = breadcrumb::read_breadcrumb(cwd) {
			return Self::open(&file_path, session_dir);
		}

		// Fall back to scanning the session directory.
		let dir = match session_dir {
			Some(p) => p.to_path_buf(),
			None => get_default_session_dir(cwd),
		};

		let storage = storage::FileSessionStorage;
		let mut files = storage.list_files(&dir, ".jsonl")?;
		if files.is_empty() {
			bail!("No sessions found in {}", dir.display());
		}

		// Sort by modification time, most recent first.
		files.sort_by(|a, b| {
			let ma = storage
				.stat(a)
				.map_or(std::time::SystemTime::UNIX_EPOCH, |s| s.modified);
			let mb = storage
				.stat(b)
				.map_or(std::time::SystemTime::UNIX_EPOCH, |s| s.modified);
			mb.cmp(&ma)
		});

		Self::open(&files[0], Some(&dir))
	}

	/// List sessions for a specific working directory.
	///
	/// Scans the session directory for `.jsonl` files, extracts summary info
	/// from each, and returns them sorted by modified time (most recent first).
	pub fn list(cwd: &Path, session_dir: Option<&Path>) -> Result<Vec<SessionInfo>> {
		let dir = match session_dir {
			Some(p) => p.to_path_buf(),
			None => get_default_session_dir(cwd),
		};

		let storage = FileSessionStorage;
		let files = storage.list_files(&dir, ".jsonl")?;

		let mut infos: Vec<SessionInfo> = files
			.iter()
			.filter_map(|f| collect_session_info(f, &storage))
			.collect();

		// Sort by modified descending (most recent first).
		infos.sort_by(|a, b| b.modified.cmp(&a.modified));
		Ok(infos)
	}

	/// List all sessions across all project directories.
	///
	/// Scans every subdirectory of the sessions base directory
	/// (`~/.rho/agent/sessions/`), collecting `.jsonl` files from each.
	/// Returns all sessions sorted by modified time (most recent first).
	pub fn list_all(session_dir: Option<&Path>) -> Result<Vec<SessionInfo>> {
		let base = match session_dir {
			Some(p) => p.to_path_buf(),
			None => get_default_agent_dir().join("sessions"),
		};

		let storage = FileSessionStorage;
		let subdirs = storage.list_dirs(&base)?;

		let mut infos: Vec<SessionInfo> = Vec::new();
		for subdir in &subdirs {
			if let Ok(files) = storage.list_files(subdir, ".jsonl") {
				for file in &files {
					if let Some(info) = collect_session_info(file, &storage) {
						infos.push(info);
					}
				}
			}
		}

		// Sort by modified descending (most recent first).
		infos.sort_by(|a, b| b.modified.cmp(&a.modified));
		Ok(infos)
	}
}

/// Extract [`SessionInfo`] from a JSONL file by reading only the first ~4KB.
///
/// Parses the header (first line) for id, title, and cwd. Counts lines in
/// the prefix for an approximate message count. Extracts the first user
/// message text for a preview. Uses file stat for created/modified timestamps.
fn collect_session_info(path: &Path, storage: &dyn SessionStorage) -> Option<SessionInfo> {
	let prefix = storage.read_text_prefix(path, 4096).ok()?;
	let stat = storage.stat(path).ok()?;

	let mut lines = prefix.lines();

	// First line is the header.
	let header_line = lines.next()?;
	let header: SessionHeader = serde_json::from_str(header_line).ok()?;

	// Count remaining lines and find first user message.
	let mut message_count = 0usize;
	let mut first_message = String::new();

	for line in lines {
		let line = line.trim();
		if line.is_empty() {
			continue;
		}
		message_count += 1;

		// Try to extract the first user message for preview.
		if first_message.is_empty() {
			if let Ok(file_entry) = serde_json::from_str::<FileEntry>(line) {
				if let FileEntry::Entry(SessionEntry::Message(ref msg_entry)) = file_entry {
					if let Message::User(ref user_msg) = msg_entry.message {
						first_message = user_msg.content.clone();
					}
				}
			}
		}
	}

	// Parse created timestamp from header.
	let created = chrono::DateTime::parse_from_rfc3339(&header.timestamp)
		.ok()
		.map(|dt| dt.with_timezone(&chrono::Utc))
		.unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from(stat.modified));

	let modified = chrono::DateTime::<chrono::Utc>::from(stat.modified);

	Some(SessionInfo {
		path: path.to_path_buf(),
		id: header.id,
		cwd: header.cwd,
		title: header.title,
		parent_session_path: header.parent_session,
		created,
		modified,
		message_count,
		first_message,
	})
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

impl SessionManager {
	/// Append a message entry.
	pub fn append_message(&mut self, message: Message) -> Result<()> {
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::Message(SessionMessageEntry {
			id,
			parent_id: self.leaf_id.clone(),
			timestamp: now,
			message: message.clone(),
		});
		self.has_message_entry = true;
		self.messages_cache.push(message);
		self.append_entry(entry)
	}

	/// Append a thinking level change entry.
	pub fn append_thinking_level_change(&mut self, level: &str) -> Result<()> {
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
			id,
			parent_id: self.leaf_id.clone(),
			timestamp: now,
			thinking_level: level.to_owned(),
		});
		self.append_entry(entry)
	}

	/// Append a model change entry.
	pub fn append_model_change(&mut self, model: &str, role: Option<&str>) -> Result<()> {
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::ModelChange(ModelChangeEntry {
			id,
			parent_id: self.leaf_id.clone(),
			timestamp: now,
			model: model.to_owned(),
			role: role.map(|r| r.to_owned()),
		});
		self.append_entry(entry)
	}

	/// Append a custom entry.
	pub fn append_custom(&mut self, custom_type: &str, data: Option<Value>) -> Result<()> {
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::Custom(CustomEntry {
			id,
			parent_id: self.leaf_id.clone(),
			timestamp: now,
			custom_type: custom_type.to_owned(),
			data,
		});
		self.append_entry(entry)
	}

	/// Append a mode change entry.
	pub fn append_mode_change(
		&mut self,
		mode: &str,
		data: Option<serde_json::Map<String, Value>>,
	) -> Result<()> {
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::ModeChange(ModeChangeEntry {
			id,
			parent_id: self.leaf_id.clone(),
			timestamp: now,
			mode: mode.to_owned(),
			data,
		});
		self.append_entry(entry)
	}

	/// Append a compaction entry that summarizes older messages.
	///
	/// After appending, rebuilds the messages cache so subsequent
	/// `messages()` / `build_context()` calls return the compacted state.
	pub fn append_compaction(
		&mut self,
		summary: &str,
		short_summary: Option<&str>,
		first_kept_entry_id: &str,
		tokens_before: u64,
		details: Option<serde_json::Value>,
	) -> Result<()> {
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::Compaction(CompactionEntry {
			id,
			parent_id:           self.leaf_id.clone(),
			timestamp:           now,
			summary:             summary.to_owned(),
			short_summary:       short_summary.map(|s| s.to_owned()),
			first_kept_entry_id: first_kept_entry_id.to_owned(),
			tokens_before,
			details,
			preserve_data:       None,
			from_extension:      None,
		});
		self.append_entry(entry)?;
		self.rebuild_messages_cache();
		Ok(())
	}

	/// Generic internal append: index, update leaf, persist.
	fn append_entry(&mut self, entry: SessionEntry) -> Result<()> {
		self.persist(&entry)?;
		let id = self.index_entry(entry);
		self.leaf_id = Some(id);
		self.is_dirty = true;
		Ok(())
	}

	/// Index an entry into the internal data structures (without persisting).
	///
	/// Returns the entry's ID for use by callers.
	fn index_entry(&mut self, entry: SessionEntry) -> String {
		let id = entry.id().to_owned();
		let idx = self.entries.len();
		self.entry_ids.insert(id.clone());
		self.by_id.insert(id.clone(), idx);
		self.entries.push(entry);
		id
	}

	/// Persist a single entry to the writer (creates the file lazily).
	///
	/// Before writing, each entry is passed through
	/// [`prepare_entry_for_persistence`] to truncate large strings and
	/// externalize image data. The original in-memory entry is untouched.
	fn persist(&mut self, entry: &SessionEntry) -> Result<()> {
		// In-memory sessions: no file, no persistence.
		let Some(ref file_path) = self.session_file else {
			return Ok(());
		};

		// Delayed persistence: don't create the file until we have a message entry.
		if !self.has_message_entry && !matches!(entry, SessionEntry::Message(_)) {
			return Ok(());
		}

		// Create the writer if not yet initialized.
		if self.writer.is_none() {
			// Ensure session directory exists.
			if let Some(ref dir) = self.session_dir {
				self.storage.ensure_dir(dir)?;
			}

			let mut writer = self.storage.open_writer(file_path, false)?;

			// Write the header as the first line.
			let header_json = serde_json::to_string(&self.header)?;
			writer.write_line(&header_json)?;

			// Write any entries that were buffered before the writer was created.
			for existing in &self.entries {
				let prepared = prepare_entry_for_persistence(existing, self.blob_store.as_ref())?;
				let line = serde_json::to_string(&prepared)?;
				writer.write_line(&line)?;
			}
			writer.flush()?;

			self.writer = Some(writer);
		}

		// Prepare and append the new entry.
		let prepared = prepare_entry_for_persistence(entry, self.blob_store.as_ref())?;
		let line = serde_json::to_string(&prepared)?;
		let writer = self.writer.as_mut().expect("writer initialized above");
		writer.write_line(&line)?;
		writer.flush()?;

		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Fork & branching
// ---------------------------------------------------------------------------

impl SessionManager {
	/// Fork the current session: create a new session file with the same
	/// entries.
	///
	/// Generates a new session ID, creates a new JSONL file with a fresh header
	/// and all existing entries, closes the old writer, and opens a new one
	/// pointing at the new file. Returns the old and new file paths.
	pub fn fork(&mut self) -> Result<ForkResult> {
		let old_file = self.session_file.clone();
		let new_session_id = snowflake::next();

		// Build a new header, preserving everything except the ID and timestamp.
		let new_header = SessionHeader {
			r#type:         self.header.r#type.clone(),
			version:        self.header.version,
			id:             new_session_id.clone(),
			title:          self.header.title.clone(),
			timestamp:      Utc::now().to_rfc3339(),
			cwd:            self.header.cwd.clone(),
			parent_session: self.header.parent_session.clone(),
		};

		// Determine the new file path.
		let new_file_name = session_file_name(&new_session_id);
		let new_file = self
			.session_dir
			.as_ref()
			.map(|dir| dir.join(&new_file_name));

		// Close the old writer.
		if let Some(ref mut w) = self.writer {
			let _ = w.close();
		}
		self.writer = None;

		// Update internal state to the new session.
		self.header = new_header;
		self.session_file = new_file.clone();

		// Create the new file and write header + all entries.
		if let Some(ref file_path) = self.session_file {
			if let Some(ref dir) = self.session_dir {
				self.storage.ensure_dir(dir)?;
			}

			let mut writer = self.storage.open_writer(file_path, false)?;

			// Write the header.
			let header_json = serde_json::to_string(&self.header)?;
			writer.write_line(&header_json)?;

			// Write all existing entries.
			for entry in &self.entries {
				let line = serde_json::to_string(entry)?;
				writer.write_line(&line)?;
			}
			writer.flush()?;

			self.writer = Some(writer);
		}

		Ok(ForkResult { old_session_file: old_file, new_session_file: self.session_file.clone() })
	}

	/// Fork from an external session file into a new working directory.
	///
	/// Opens the source session, creates a brand-new session with a new ID and
	/// the given `cwd`, copies all entries from the source, and sets
	/// `parent_session` to the source session's ID.
	pub fn fork_from(source: &Path, cwd: &Path, session_dir: Option<&Path>) -> Result<Self> {
		// Open the source session to read its entries.
		let source_mgr = Self::open(source, None)?;
		let source_id = source_mgr.session_id().to_owned();

		// Create a new session in the target CWD.
		let mut new_mgr = Self::create(cwd, session_dir)?;

		// Set parent_session to the source session's ID.
		new_mgr.header.parent_session = Some(source_id);

		// Copy all entries from the source, preserving their IDs and structure.
		for entry in &source_mgr.entries {
			let id = new_mgr.index_entry(entry.clone());

			// Track if we've seen a message entry (for delayed persistence).
			if matches!(entry, SessionEntry::Message(_)) {
				new_mgr.has_message_entry = true;
			}

			// Update entry_ids is already done by index_entry; update leaf.
			new_mgr.leaf_id = Some(id);
		}

		// Find the proper leaf (the source may have branches).
		new_mgr.leaf_id = new_mgr.find_leaf();

		// Rebuild the messages cache.
		new_mgr.rebuild_messages_cache();

		Ok(new_mgr)
	}

	/// Branch: move the leaf to a previous entry and append a summary of the
	/// abandoned path.
	///
	/// Verifies that `from_id` exists, moves the leaf pointer to it, then
	/// appends a [`BranchSummaryEntry`] with the given summary text. The
	/// `from_id` field in the entry records where the branch happened.
	pub fn branch_with_summary(&mut self, from_id: &str, summary: &str) -> Result<()> {
		// Verify from_id exists.
		if !self.by_id.contains_key(from_id) {
			bail!("Entry not found: {from_id}");
		}

		// Move leaf to from_id.
		self.leaf_id = Some(from_id.to_owned());

		// Create and append a BranchSummary entry.
		let now = Utc::now().to_rfc3339();
		let id = snowflake::generate_entry_id(&self.entry_ids);
		let entry = SessionEntry::BranchSummary(BranchSummaryEntry {
			id,
			parent_id: Some(from_id.to_owned()),
			timestamp: now,
			from_id: from_id.to_owned(),
			summary: summary.to_owned(),
			details: None,
			from_extension: None,
		});

		self.append_entry(entry)?;

		// Rebuild messages cache since the branch changed.
		self.rebuild_messages_cache();

		Ok(())
	}

	/// Move the leaf pointer to a specific entry (for branching).
	///
	/// Verifies that the entry exists in the session, updates `leaf_id`,
	/// and rebuilds the messages cache to reflect the new branch.
	pub fn move_to(&mut self, entry_id: &str) -> Result<()> {
		if !self.by_id.contains_key(entry_id) {
			bail!("Entry not found: {entry_id}");
		}

		self.leaf_id = Some(entry_id.to_owned());
		self.rebuild_messages_cache();

		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Tree queries
// ---------------------------------------------------------------------------

impl SessionManager {
	/// Get an entry by ID.
	pub fn get_entry(&self, id: &str) -> Option<&SessionEntry> {
		self.by_id.get(id).map(|&idx| &self.entries[idx])
	}

	/// Get the current leaf entry ID (tip of the active branch).
	pub fn leaf_id(&self) -> Option<&str> {
		self.leaf_id.as_deref()
	}

	/// Get the session header.
	pub const fn header(&self) -> &SessionHeader {
		&self.header
	}

	/// Get the session ID (snowflake).
	pub fn session_id(&self) -> &str {
		&self.header.id
	}

	/// Set the session title.
	pub fn set_title(&mut self, title: &str) {
		self.header.title = Some(title.to_owned());
	}

	/// Get all entries from `from_id` to root (the branch path).
	///
	/// Returns entries in leaf-to-root order.
	pub fn get_branch(&self, from_id: &str) -> Vec<&SessionEntry> {
		let mut branch = Vec::new();
		let mut current_id: Option<&str> = Some(from_id);

		while let Some(id) = current_id {
			if let Some(entry) = self.get_entry(id) {
				branch.push(entry);
				current_id = entry.parent_id();
			} else {
				break;
			}
		}

		branch
	}

	/// Get child entries of a given parent.
	///
	/// If `parent_id` is `None`, returns root entries (those with no parent).
	pub fn get_children(&self, parent_id: Option<&str>) -> Vec<&SessionEntry> {
		self
			.entries
			.iter()
			.filter(|e| e.parent_id() == parent_id)
			.collect()
	}

	/// All entries in file order.
	pub fn entries(&self) -> &[SessionEntry] {
		&self.entries
	}

	/// Whether the session has unsaved changes.
	pub const fn is_dirty(&self) -> bool {
		self.is_dirty
	}

	/// The session file path (if persisted).
	pub fn session_file(&self) -> Option<&Path> {
		self.session_file.as_deref()
	}

	/// Build the conversation context from the current branch.
	///
	/// Walks from the leaf to root, then delegates to [`context::build_context`]
	/// to reconstruct messages, thinking level, models, and other metadata.
	///
	/// Returns a default (empty) [`SessionContext`] if there is no leaf.
	pub fn build_context(&self) -> types::SessionContext {
		let branch = match self.leaf_id() {
			Some(id) => self.get_branch(id),
			None => Vec::new(),
		};
		context::build_context(&branch)
	}
}

// ---------------------------------------------------------------------------
// Entry persistence preparation
// ---------------------------------------------------------------------------

/// Maximum string length before truncation (100,000 characters).
const MAX_STRING_LENGTH: usize = 100_000;

/// Minimum base64 data length to trigger image externalization (1,024
/// characters).
const MIN_IMAGE_EXTERNALIZE_LENGTH: usize = 1_024;

/// Truncation suffix appended to strings that exceed [`MAX_STRING_LENGTH`].
const TRUNCATION_SUFFIX: &str = "\n... (truncated)";

/// Prepare an entry for persistence: truncate large strings, externalize
/// images.
///
/// Works on a clone of the entry so the original remains untouched.
fn prepare_entry_for_persistence(
	entry: &SessionEntry,
	blob_store: Option<&BlobStore>,
) -> Result<SessionEntry> {
	let mut value = serde_json::to_value(entry)?;
	truncate_for_persistence(&mut value, blob_store)?;
	let prepared: SessionEntry = serde_json::from_value(value)?;
	Ok(prepared)
}

/// Recursively truncate large string values in a [`serde_json::Value`].
///
/// Applies the following transformations:
/// - Strings > 100,000 chars: truncated to 100,000 + `"\n... (truncated)"`
///   suffix
/// - Image content blocks (`type: "image"` with `source.data` > 1,024 chars):
///   externalize base64 data to blob store
/// - `partialJson` fields: removed entirely (streaming artifact)
/// - `jsonlEvents` fields: removed entirely (streaming artifact)
fn truncate_for_persistence(value: &mut Value, blob_store: Option<&BlobStore>) -> Result<()> {
	match value {
		Value::String(s) => {
			if s.len() > MAX_STRING_LENGTH {
				let boundary = s.floor_char_boundary(MAX_STRING_LENGTH);
				s.truncate(boundary);
				s.push_str(TRUNCATION_SUFFIX);
			}
		},
		Value::Array(arr) => {
			for item in arr.iter_mut() {
				truncate_for_persistence(item, blob_store)?;
			}
		},
		Value::Object(map) => {
			// Remove streaming artifact fields.
			map.remove("partialJson");
			map.remove("jsonlEvents");

			// Check if this is an image content block with base64 data to externalize.
			let is_image = map
				.get("type")
				.and_then(Value::as_str)
				.is_some_and(|t| t == "image");

			if is_image
				&& let Some(source) = map.get_mut("source")
				&& let Some(data) = source.get_mut("data")
				&& let Some(data_str) = data.as_str()
				&& data_str.len() > MIN_IMAGE_EXTERNALIZE_LENGTH
				&& let Some(store) = blob_store
				&& !blob_store::is_blob_ref(data_str)
			{
				let blob_ref = blob_store::externalize_image_data(store, data_str)?;
				*data = Value::String(blob_ref);
			}

			// Recurse into all remaining values.
			for val in map.values_mut() {
				truncate_for_persistence(val, blob_store)?;
			}
		},
		_ => {},
	}
	Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl SessionManager {
	/// Parse JSONL content into a header and entries.
	///
	/// Tolerates a malformed last line (likely a partial write from a crash)
	/// by skipping it with a warning. Mid-file corruption still returns an
	/// error so that data integrity issues are surfaced.
	fn parse_jsonl(content: &str) -> Result<(SessionHeader, Vec<SessionEntry>)> {
		let mut lines = content.lines();

		// First line must be the header.
		let header_line = lines
			.next()
			.ok_or_else(|| anyhow::anyhow!("Empty session file"))?;
		let file_entry: FileEntry = serde_json::from_str(header_line)?;
		let header = match file_entry {
			FileEntry::Header(h) => h,
			FileEntry::Entry(_) => bail!("Expected session header, got entry"),
		};

		// Collect remaining non-empty lines.
		let remaining: Vec<&str> = lines
			.map(|l| l.trim())
			.filter(|l| !l.is_empty())
			.collect();

		let mut entries = Vec::new();
		let last_idx = remaining.len().saturating_sub(1);

		for (i, line) in remaining.iter().enumerate() {
			match serde_json::from_str::<FileEntry>(line) {
				Ok(FileEntry::Entry(entry)) => entries.push(entry),
				Ok(FileEntry::Header(_)) => bail!("Unexpected second header in session file"),
				Err(e) if i == last_idx && !remaining.is_empty() => {
					eprintln!(
						"Warning: skipping malformed last entry in session file \
						 (possible partial write): {e}"
					);
				},
				Err(e) => return Err(e.into()),
			}
		}

		Ok((header, entries))
	}

	/// Find the leaf entry: the last entry that has no children.
	///
	/// Walk from the last entry backwards. In a linear conversation, this is
	/// simply the last entry. In a branching conversation, we find the entry
	/// that is not a parent of any other entry.
	fn find_leaf(&self) -> Option<String> {
		if self.entries.is_empty() {
			return None;
		}

		// Collect all IDs that are parents of other entries.
		let mut parent_ids: HashSet<&str> = HashSet::new();
		for entry in &self.entries {
			if let Some(pid) = entry.parent_id() {
				parent_ids.insert(pid);
			}
		}

		// Walk backwards to find the last entry that is not a parent.
		for entry in self.entries.iter().rev() {
			if !parent_ids.contains(entry.id()) {
				return Some(entry.id().to_owned());
			}
		}

		// Fallback: last entry.
		self.entries.last().map(|e| e.id().to_owned())
	}

	/// Rebuild the flat messages cache from entries on the current branch.
	///
	/// Delegates to [`build_context()`] so the cache is compaction-aware:
	/// when a compaction entry exists, older messages are replaced by the
	/// summary.
	fn rebuild_messages_cache(&mut self) {
		let ctx = self.build_context();
		self.messages_cache = ctx.messages;
	}
}

// ---------------------------------------------------------------------------
// Compatibility shims (old API used by interactive.rs, commands)
// ---------------------------------------------------------------------------

impl SessionManager {
	/// Resume a specific session by path.
	pub fn resume(session_path: &Path) -> Self {
		// Try to find a JSONL file at the given path.
		if session_path.is_file() {
			return Self::open(session_path, None).unwrap_or_else(|_| Self::in_memory());
		}
		// If it's a directory, look for the most recent JSONL file inside.
		if session_path.is_dir() {
			let storage = storage::FileSessionStorage;
			if let Ok(mut files) = storage.list_files(session_path, ".jsonl") {
				files.sort_by(|a, b| {
					let ma = storage
						.stat(a)
						.map_or(std::time::SystemTime::UNIX_EPOCH, |s| s.modified);
					let mb = storage
						.stat(b)
						.map_or(std::time::SystemTime::UNIX_EPOCH, |s| s.modified);
					mb.cmp(&ma)
				});
				if let Some(file) = files.first() {
					return Self::open(file, Some(session_path)).unwrap_or_else(|_| Self::in_memory());
				}
			}
		}
		Self::in_memory()
	}

	/// Load messages from storage (old API, now a no-op since `open()` loads).
	#[allow(
		clippy::unused_async,
		clippy::future_not_send,
		reason = "async signature kept for compatibility with callers; SessionManager is !Sync"
	)]
	pub async fn load(&self) -> Result<()> {
		Ok(())
	}

	/// Get messages as a flat slice (old API).
	pub fn messages(&self) -> &[Message] {
		&self.messages_cache
	}

	/// Clear all messages and reset the session (old API).
	#[allow(clippy::unused_async, reason = "async signature kept for compatibility with callers")]
	pub async fn clear(&mut self) -> Result<()> {
		self.entries.clear();
		self.by_id.clear();
		self.labels_by_id.clear();
		self.leaf_id = None;
		self.entry_ids.clear();
		self.messages_cache.clear();
		self.has_message_entry = false;
		self.is_dirty = false;
		// Close the existing writer; a new file will be created on next append.
		if let Some(ref mut w) = self.writer {
			let _ = w.close();
		}
		self.writer = None;
		Ok(())
	}

	/// Append a message and persist to storage (old API).
	#[allow(clippy::unused_async, reason = "async signature kept for compatibility with callers")]
	pub async fn append(&mut self, message: Message) -> Result<()> {
		self.append_message(message)
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ai::types::{Message, UserMessage};

	// -- Compatibility tests (keep existing behavior working) --

	#[tokio::test]
	async fn test_session_manager_new() {
		let mut mgr = SessionManager::in_memory();
		let msg = Message::User(UserMessage { content: "Test".to_owned() });
		mgr.append(msg).await.unwrap();
		assert_eq!(mgr.messages().len(), 1);
	}

	#[tokio::test]
	async fn test_in_memory_session() {
		let mut mgr = SessionManager::in_memory();
		let msg = Message::User(UserMessage { content: "Test".to_owned() });
		mgr.append(msg).await.unwrap();
		assert_eq!(mgr.messages().len(), 1);
	}

	#[tokio::test]
	async fn test_clear_session() {
		let mut mgr = SessionManager::in_memory();
		let id_before = mgr.session_id().to_owned();

		mgr.append(Message::User(UserMessage { content: "Hello".to_owned() }))
			.await
			.unwrap();
		mgr.append(Message::User(UserMessage { content: "World".to_owned() }))
			.await
			.unwrap();
		assert_eq!(mgr.messages().len(), 2);

		mgr.clear().await.unwrap();
		assert!(mgr.messages().is_empty());
		assert_eq!(mgr.session_id(), id_before);
	}

	// -- New task-9 tests --

	#[test]
	fn test_create_session() {
		let tmp = tempfile::tempdir().unwrap();
		let cwd = tmp.path().join("project");
		std::fs::create_dir_all(&cwd).unwrap();

		let mgr = SessionManager::create(&cwd, Some(tmp.path())).unwrap();

		assert_eq!(mgr.header().r#type, "session");
		assert_eq!(mgr.header().version, 1);
		assert!(snowflake::valid(mgr.session_id()));
		assert_eq!(mgr.header().cwd, cwd.to_string_lossy());
		assert!(mgr.header().title.is_none());
		assert!(mgr.header().parent_session.is_none());
		assert!(mgr.entries().is_empty());
		assert!(mgr.leaf_id().is_none());
	}

	#[test]
	fn test_append_message_updates_leaf() {
		let mut mgr = SessionManager::in_memory();

		assert!(mgr.leaf_id().is_none());

		mgr.append_message(Message::User(UserMessage { content: "Hello".to_owned() }))
			.unwrap();

		assert!(mgr.leaf_id().is_some());
		let leaf = mgr.leaf_id().unwrap().to_owned();

		// The leaf should be the entry we just appended.
		let entry = mgr.get_entry(&leaf).unwrap();
		assert!(matches!(entry, SessionEntry::Message(_)));

		// Append another message; leaf should update.
		mgr.append_message(Message::User(UserMessage { content: "World".to_owned() }))
			.unwrap();

		let new_leaf = mgr.leaf_id().unwrap();
		assert_ne!(new_leaf, leaf.as_str(), "leaf should change after second append");
	}

	#[test]
	fn test_append_builds_tree() {
		let mut mgr = SessionManager::in_memory();

		// Append three messages.
		mgr.append_message(Message::User(UserMessage { content: "A".to_owned() }))
			.unwrap();
		let id_a = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "B".to_owned() }))
			.unwrap();
		let id_b = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "C".to_owned() }))
			.unwrap();
		let id_c = mgr.leaf_id().unwrap().to_owned();

		// Entry A has no parent.
		let entry_a = mgr.get_entry(&id_a).unwrap();
		assert!(entry_a.parent_id().is_none());

		// Entry B's parent is A.
		let entry_b = mgr.get_entry(&id_b).unwrap();
		assert_eq!(entry_b.parent_id(), Some(id_a.as_str()));

		// Entry C's parent is B.
		let entry_c = mgr.get_entry(&id_c).unwrap();
		assert_eq!(entry_c.parent_id(), Some(id_b.as_str()));
	}

	#[test]
	fn test_get_branch_returns_path() {
		let mut mgr = SessionManager::in_memory();

		mgr.append_message(Message::User(UserMessage { content: "A".to_owned() }))
			.unwrap();
		let id_a = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "B".to_owned() }))
			.unwrap();
		let id_b = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "C".to_owned() }))
			.unwrap();
		let id_c = mgr.leaf_id().unwrap().to_owned();

		// Get branch from C to root.
		let branch = mgr.get_branch(&id_c);
		assert_eq!(branch.len(), 3);

		// Branch is leaf-to-root: [C, B, A].
		assert_eq!(branch[0].id(), id_c);
		assert_eq!(branch[1].id(), id_b);
		assert_eq!(branch[2].id(), id_a);
	}

	#[test]
	fn test_open_restores_state() {
		let tmp = tempfile::tempdir().unwrap();
		let session_dir = tmp.path();
		let cwd = tmp.path().join("project");
		std::fs::create_dir_all(&cwd).unwrap();

		// Create a session and append messages.
		let mut mgr = SessionManager::create(&cwd, Some(session_dir)).unwrap();
		mgr.append_message(Message::User(UserMessage { content: "Hello".to_owned() }))
			.unwrap();
		mgr.append_message(Message::User(UserMessage { content: "World".to_owned() }))
			.unwrap();

		let original_id = mgr.session_id().to_owned();
		let original_leaf = mgr.leaf_id().unwrap().to_owned();
		let file_path = mgr.session_file().unwrap().to_path_buf();

		// Open the same file.
		let restored = SessionManager::open(&file_path, Some(session_dir)).unwrap();

		assert_eq!(restored.session_id(), original_id);
		assert_eq!(restored.entries().len(), 2);
		assert_eq!(restored.leaf_id(), Some(original_leaf.as_str()));
		assert_eq!(restored.messages().len(), 2);

		// Verify message content.
		match &restored.messages()[0] {
			Message::User(u) => assert_eq!(u.content, "Hello"),
			_ => panic!("Expected User message"),
		}
		match &restored.messages()[1] {
			Message::User(u) => assert_eq!(u.content, "World"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_in_memory_no_persistence() {
		let mut mgr = SessionManager::in_memory();

		// Append works.
		mgr.append_message(Message::User(UserMessage { content: "test".to_owned() }))
			.unwrap();
		assert_eq!(mgr.entries().len(), 1);
		assert_eq!(mgr.messages().len(), 1);

		// No file created.
		assert!(mgr.session_file().is_none());
		assert!(mgr.writer.is_none());
	}

	#[test]
	fn test_set_title() {
		let mut mgr = SessionManager::in_memory();
		assert!(mgr.header().title.is_none());

		mgr.set_title("My Session");
		assert_eq!(mgr.header().title.as_deref(), Some("My Session"));

		mgr.set_title("Updated Title");
		assert_eq!(mgr.header().title.as_deref(), Some("Updated Title"));
	}

	#[test]
	fn test_session_id_is_snowflake() {
		let mgr = SessionManager::in_memory();
		let id = mgr.session_id();
		assert!(snowflake::valid(id), "Session ID should be a valid snowflake, got: {id}");
		assert_eq!(id.len(), 16);
		assert!(
			id.chars()
				.all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
		);
	}

	// -- Task 11: Session listing tests --

	/// Helper: build a minimal JSONL session string with a header and one user
	/// message.
	fn make_session_jsonl(
		session_id: &str,
		cwd: &str,
		title: Option<&str>,
		user_msg: &str,
	) -> String {
		let header = serde_json::json!({
			 "type": "session",
			 "version": 1,
			 "id": session_id,
			 "timestamp": "2026-01-15T10:30:00+00:00",
			 "cwd": cwd,
			 "title": title,
		});
		let entry = serde_json::json!({
			 "type": "message",
			 "id": "aa000001",
			 "timestamp": "2026-01-15T10:31:00+00:00",
			 "message": {
				  "role": "user",
				  "content": user_msg,
			 }
		});
		format!("{}\n{}\n", header, entry)
	}

	#[test]
	fn test_list_empty_dir() {
		let tmp = tempfile::tempdir().unwrap();
		let session_dir = tmp.path().join("sessions");
		std::fs::create_dir_all(&session_dir).unwrap();

		let cwd = Path::new("/home/user/project");
		let result = SessionManager::list(cwd, Some(&session_dir)).unwrap();
		assert!(result.is_empty());
	}

	#[test]
	fn test_list_finds_sessions() {
		let tmp = tempfile::tempdir().unwrap();
		let session_dir = tmp.path().join("sessions");
		std::fs::create_dir_all(&session_dir).unwrap();

		// Write two session files.
		let content1 =
			make_session_jsonl("0000000000000001", "/home/user/project", Some("Session 1"), "Hello");
		let content2 =
			make_session_jsonl("0000000000000002", "/home/user/project", Some("Session 2"), "World");
		std::fs::write(session_dir.join("session1.jsonl"), &content1).unwrap();
		std::fs::write(session_dir.join("session2.jsonl"), &content2).unwrap();

		let cwd = Path::new("/home/user/project");
		let result = SessionManager::list(cwd, Some(&session_dir)).unwrap();
		assert_eq!(result.len(), 2);

		// Both sessions should be found.
		let ids: Vec<&str> = result.iter().map(|s| s.id.as_str()).collect();
		assert!(ids.contains(&"0000000000000001"));
		assert!(ids.contains(&"0000000000000002"));
	}

	#[test]
	fn test_list_sorted_by_mtime() {
		let tmp = tempfile::tempdir().unwrap();
		let session_dir = tmp.path().join("sessions");
		std::fs::create_dir_all(&session_dir).unwrap();

		// Write first session, then sleep briefly, then second.
		let content1 = make_session_jsonl("0000000000000001", "/home/user/project", None, "First");
		let content2 = make_session_jsonl("0000000000000002", "/home/user/project", None, "Second");

		std::fs::write(session_dir.join("old.jsonl"), &content1).unwrap();
		// Touch the file to make it older by setting an old mtime.
		let old_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
		filetime::set_file_mtime(
			session_dir.join("old.jsonl"),
			filetime::FileTime::from_system_time(old_time),
		)
		.unwrap();

		std::fs::write(session_dir.join("new.jsonl"), &content2).unwrap();

		let cwd = Path::new("/home/user/project");
		let result = SessionManager::list(cwd, Some(&session_dir)).unwrap();
		assert_eq!(result.len(), 2);

		// Most recent first.
		assert_eq!(result[0].id, "0000000000000002");
		assert_eq!(result[1].id, "0000000000000001");
	}

	#[test]
	fn test_list_all_multiple_cwds() {
		let tmp = tempfile::tempdir().unwrap();
		let base_dir = tmp.path().join("sessions");

		// Create two subdirs (simulating two different CWDs).
		let dir_a = base_dir.join("project-a");
		let dir_b = base_dir.join("project-b");
		std::fs::create_dir_all(&dir_a).unwrap();
		std::fs::create_dir_all(&dir_b).unwrap();

		let content_a = make_session_jsonl("0000000000000001", "/home/user/project-a", None, "Alpha");
		let content_b = make_session_jsonl("0000000000000002", "/home/user/project-b", None, "Beta");
		std::fs::write(dir_a.join("a.jsonl"), &content_a).unwrap();
		std::fs::write(dir_b.join("b.jsonl"), &content_b).unwrap();

		let result = SessionManager::list_all(Some(&base_dir)).unwrap();
		assert_eq!(result.len(), 2);

		let ids: Vec<&str> = result.iter().map(|s| s.id.as_str()).collect();
		assert!(ids.contains(&"0000000000000001"));
		assert!(ids.contains(&"0000000000000002"));

		// Check that CWDs are preserved.
		let cwds: Vec<&str> = result.iter().map(|s| s.cwd.as_str()).collect();
		assert!(cwds.contains(&"/home/user/project-a"));
		assert!(cwds.contains(&"/home/user/project-b"));
	}

	#[test]
	fn test_collect_session_info_parses_header() {
		let tmp = tempfile::tempdir().unwrap();
		let session_file = tmp.path().join("test.jsonl");

		let content = make_session_jsonl(
			"abcdef0123456789",
			"/home/user/myproject",
			Some("My Title"),
			"What is Rust?",
		);
		std::fs::write(&session_file, &content).unwrap();

		let storage = storage::FileSessionStorage;
		let info = collect_session_info(&session_file, &storage).expect("should parse session info");

		assert_eq!(info.id, "abcdef0123456789");
		assert_eq!(info.cwd, "/home/user/myproject");
		assert_eq!(info.title.as_deref(), Some("My Title"));
		assert_eq!(info.first_message, "What is Rust?");
		assert_eq!(info.message_count, 1);
		assert_eq!(info.path, session_file);
	}

	// -- Task 12: Fork and branching tests --

	#[test]
	fn test_fork_creates_new_session() {
		let tmp = tempfile::tempdir().unwrap();
		let cwd = tmp.path().join("project");
		std::fs::create_dir_all(&cwd).unwrap();

		let mut mgr = SessionManager::create(&cwd, Some(tmp.path())).unwrap();
		let old_id = mgr.session_id().to_owned();

		// Append some messages so the session has content.
		mgr.append_message(Message::User(UserMessage { content: "Hello".to_owned() }))
			.unwrap();
		mgr.append_message(Message::User(UserMessage { content: "World".to_owned() }))
			.unwrap();

		let entry_count_before = mgr.entries().len();
		assert_eq!(entry_count_before, 2);

		let result = mgr.fork().unwrap();

		// New session ID should differ from the old one.
		assert_ne!(mgr.session_id(), old_id);

		// Entries should be preserved.
		assert_eq!(mgr.entries().len(), entry_count_before);

		// Both old and new file paths should be present.
		assert!(result.old_session_file.is_some());
		assert!(result.new_session_file.is_some());
		assert_ne!(result.old_session_file, result.new_session_file);

		// The new file should exist on disk.
		let new_file = result.new_session_file.unwrap();
		assert!(new_file.exists(), "New session file should exist on disk");
	}

	#[test]
	fn test_fork_from_copies_entries() {
		let tmp = tempfile::tempdir().unwrap();
		let source_dir = tmp.path().join("source");
		let target_dir = tmp.path().join("target");
		std::fs::create_dir_all(&source_dir).unwrap();
		std::fs::create_dir_all(&target_dir).unwrap();

		// Create a source session with messages.
		let mut source = SessionManager::create(&source_dir, Some(&source_dir)).unwrap();
		source
			.append_message(Message::User(UserMessage { content: "Alpha".to_owned() }))
			.unwrap();
		source
			.append_message(Message::User(UserMessage { content: "Beta".to_owned() }))
			.unwrap();

		let source_file = source.session_file().unwrap().to_path_buf();
		let source_entry_count = source.entries().len();

		// Fork from the source into a new CWD.
		let forked = SessionManager::fork_from(&source_file, &target_dir, Some(&target_dir)).unwrap();

		// The forked session should have the same number of entries.
		assert_eq!(forked.entries().len(), source_entry_count);

		// Messages cache should be populated.
		assert_eq!(forked.messages().len(), 2);

		// Verify message content.
		match &forked.messages()[0] {
			Message::User(u) => assert_eq!(u.content, "Alpha"),
			_ => panic!("Expected User message"),
		}
		match &forked.messages()[1] {
			Message::User(u) => assert_eq!(u.content, "Beta"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_fork_from_sets_parent() {
		let tmp = tempfile::tempdir().unwrap();
		let source_dir = tmp.path().join("source");
		let target_dir = tmp.path().join("target");
		std::fs::create_dir_all(&source_dir).unwrap();
		std::fs::create_dir_all(&target_dir).unwrap();

		// Create a source session.
		let mut source = SessionManager::create(&source_dir, Some(&source_dir)).unwrap();
		source
			.append_message(Message::User(UserMessage { content: "Hello".to_owned() }))
			.unwrap();

		let source_id = source.session_id().to_owned();
		let source_file = source.session_file().unwrap().to_path_buf();

		// Fork from the source.
		let forked = SessionManager::fork_from(&source_file, &target_dir, Some(&target_dir)).unwrap();

		// The forked session should have parent_session set to the source ID.
		assert_eq!(forked.header().parent_session.as_deref(), Some(source_id.as_str()),);

		// The forked session should have a different session ID.
		assert_ne!(forked.session_id(), source_id);
	}

	#[test]
	fn test_branch_with_summary_moves_leaf() {
		let mut mgr = SessionManager::in_memory();

		// Append three messages: A -> B -> C.
		mgr.append_message(Message::User(UserMessage { content: "A".to_owned() }))
			.unwrap();
		let id_a = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "B".to_owned() }))
			.unwrap();

		mgr.append_message(Message::User(UserMessage { content: "C".to_owned() }))
			.unwrap();

		// Branch from A with a summary.
		mgr.branch_with_summary(&id_a, "Abandoned the B->C path")
			.unwrap();

		// The leaf should now be at the new BranchSummary entry (child of A),
		// not at C anymore.
		let leaf = mgr.leaf_id().unwrap();
		assert_ne!(leaf, id_a, "Leaf should be at the new entry, not from_id itself");

		// The leaf's parent should be A.
		let leaf_entry = mgr.get_entry(leaf).unwrap();
		assert_eq!(leaf_entry.parent_id(), Some(id_a.as_str()));
	}

	#[test]
	fn test_branch_with_summary_adds_entry() {
		let mut mgr = SessionManager::in_memory();

		mgr.append_message(Message::User(UserMessage { content: "A".to_owned() }))
			.unwrap();
		let id_a = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "B".to_owned() }))
			.unwrap();

		let entries_before = mgr.entries().len();

		mgr.branch_with_summary(&id_a, "Took a different direction")
			.unwrap();

		// One new entry should have been added.
		assert_eq!(mgr.entries().len(), entries_before + 1);

		// The last entry should be a BranchSummary.
		let last_entry = mgr.entries().last().unwrap();
		match last_entry {
			SessionEntry::BranchSummary(bs) => {
				assert_eq!(bs.from_id, id_a);
				assert_eq!(bs.summary, "Took a different direction");
				assert_eq!(bs.parent_id.as_deref(), Some(id_a.as_str()));
			},
			_ => panic!("Expected BranchSummary entry, got {:?}", last_entry),
		}
	}

	#[test]
	fn test_move_to_valid_id() {
		let mut mgr = SessionManager::in_memory();

		mgr.append_message(Message::User(UserMessage { content: "A".to_owned() }))
			.unwrap();
		let id_a = mgr.leaf_id().unwrap().to_owned();

		mgr.append_message(Message::User(UserMessage { content: "B".to_owned() }))
			.unwrap();

		mgr.append_message(Message::User(UserMessage { content: "C".to_owned() }))
			.unwrap();
		let id_c = mgr.leaf_id().unwrap().to_owned();

		// Leaf is at C.
		assert_eq!(mgr.leaf_id(), Some(id_c.as_str()));

		// Move to A.
		mgr.move_to(&id_a).unwrap();
		assert_eq!(mgr.leaf_id(), Some(id_a.as_str()));

		// Messages cache should only contain A (the branch from A to root).
		assert_eq!(mgr.messages().len(), 1);
		match &mgr.messages()[0] {
			Message::User(u) => assert_eq!(u.content, "A"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_move_to_invalid_id() {
		let mut mgr = SessionManager::in_memory();

		mgr.append_message(Message::User(UserMessage { content: "A".to_owned() }))
			.unwrap();

		// Moving to a non-existent ID should return an error.
		let result = mgr.move_to("nonexistent");
		assert!(result.is_err());
		assert!(
			result.unwrap_err().to_string().contains("Entry not found"),
			"Error message should mention entry not found"
		);
	}

	// -- Task 13: Entry persistence and truncation tests --

	#[test]
	fn test_truncate_long_string() {
		// A string longer than 100,000 chars should be truncated.
		let long_str = "x".repeat(150_000);
		let entry = SessionEntry::Message(SessionMessageEntry {
			id:        "aa000001".to_owned(),
			parent_id: None,
			timestamp: "2026-01-15T10:30:00Z".to_owned(),
			message:   Message::User(UserMessage { content: long_str.clone() }),
		});

		let prepared = prepare_entry_for_persistence(&entry, None).unwrap();
		match &prepared {
			SessionEntry::Message(msg) => match &msg.message {
				Message::User(u) => {
					assert_eq!(
						u.content.len(),
						MAX_STRING_LENGTH + TRUNCATION_SUFFIX.len(),
						"Should be truncated to MAX_STRING_LENGTH + suffix"
					);
					assert!(u.content.ends_with(TRUNCATION_SUFFIX), "Should end with truncation suffix");
					assert!(
						u.content.len() < long_str.len(),
						"Prepared content should be shorter than original"
					);
				},
				_ => panic!("Expected User message"),
			},
			_ => panic!("Expected Message entry"),
		}

		// Verify the original entry is unchanged.
		match &entry {
			SessionEntry::Message(msg) => match &msg.message {
				Message::User(u) => assert_eq!(u.content.len(), 150_000),
				_ => panic!("Expected User message"),
			},
			_ => panic!("Expected Message entry"),
		}
	}

	#[test]
	fn test_truncate_short_string() {
		// A string shorter than 100,000 chars should remain unchanged.
		let short_str = "Hello, world!".to_owned();
		let entry = SessionEntry::Message(SessionMessageEntry {
			id:        "aa000002".to_owned(),
			parent_id: None,
			timestamp: "2026-01-15T10:30:00Z".to_owned(),
			message:   Message::User(UserMessage { content: short_str.clone() }),
		});

		let prepared = prepare_entry_for_persistence(&entry, None).unwrap();
		match &prepared {
			SessionEntry::Message(msg) => match &msg.message {
				Message::User(u) => {
					assert_eq!(u.content, short_str, "Short strings should not be modified");
				},
				_ => panic!("Expected User message"),
			},
			_ => panic!("Expected Message entry"),
		}
	}

	#[test]
	fn test_truncate_multibyte_at_boundary() {
		// Build a string that places a multi-byte character right at
		// MAX_STRING_LENGTH so that a naive byte-offset truncation would
		// land inside the character and panic.
		// U+1F600 (😀) is 4 bytes in UTF-8.
		let prefix = "x".repeat(MAX_STRING_LENGTH - 1);
		let long_str = format!("{prefix}😀 and more text to exceed the limit");
		assert!(long_str.len() > MAX_STRING_LENGTH);

		let mut value = serde_json::Value::String(long_str);
		truncate_for_persistence(&mut value, None).unwrap();

		let s = value.as_str().unwrap();
		// The emoji straddles the boundary, so floor_char_boundary should
		// back up to before the emoji.
		assert!(s.len() <= MAX_STRING_LENGTH + TRUNCATION_SUFFIX.len());
		assert!(s.ends_with(TRUNCATION_SUFFIX));
		// Verify the string is valid UTF-8 (would panic above if not).
		assert!(s.is_char_boundary(s.len()));
	}

	#[test]
	fn test_externalize_image_in_entry() {
		use base64::Engine;

		use crate::session::blob_store::{BlobStore, is_blob_ref};

		let tmp = tempfile::tempdir().unwrap();
		let store = BlobStore::new(tmp.path().join("blobs"));

		// Create a large base64 string (> 1024 chars).
		let raw_bytes = vec![0xffu8; 1024];
		let b64_data = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);
		assert!(b64_data.len() > MIN_IMAGE_EXTERNALIZE_LENGTH, "base64 data should exceed threshold");

		// Build a custom entry with image-like JSON structure.
		// We use a Custom entry whose data contains an image content block.
		let image_block = serde_json::json!({
			 "type": "image",
			 "source": {
				  "type": "base64",
				  "media_type": "image/png",
				  "data": b64_data,
			 }
		});

		let entry = SessionEntry::Custom(CustomEntry {
			id:          "aa000003".to_owned(),
			parent_id:   None,
			timestamp:   "2026-01-15T10:30:00Z".to_owned(),
			custom_type: "image_test".to_owned(),
			data:        Some(serde_json::json!({
				 "content": [image_block],
			})),
		});

		let prepared = prepare_entry_for_persistence(&entry, Some(&store)).unwrap();

		// Extract the data and verify the image was externalized.
		match &prepared {
			SessionEntry::Custom(c) => {
				let data = c.data.as_ref().unwrap();
				let content = data["content"].as_array().unwrap();
				let img = &content[0];
				let new_data = img["source"]["data"].as_str().unwrap();
				assert!(
					is_blob_ref(new_data),
					"Image data should be replaced with a blob reference, got: {}",
					&new_data[..new_data.len().min(80)]
				);
			},
			_ => panic!("Expected Custom entry"),
		}

		// Verify the original entry is unchanged (still has raw base64).
		match &entry {
			SessionEntry::Custom(c) => {
				let data = c.data.as_ref().unwrap();
				let content = data["content"].as_array().unwrap();
				let img = &content[0];
				let original_data = img["source"]["data"].as_str().unwrap();
				assert_eq!(original_data, b64_data);
			},
			_ => panic!("Expected Custom entry"),
		}
	}

	#[test]
	fn test_delayed_write_no_file_until_first_entry() {
		let tmp = tempfile::tempdir().unwrap();
		let cwd = tmp.path().join("project");
		std::fs::create_dir_all(&cwd).unwrap();

		let mgr = SessionManager::create(&cwd, Some(tmp.path())).unwrap();

		// The session file path is set but the file should NOT exist yet.
		let file_path = mgr.session_file().unwrap();
		assert!(
			!file_path.exists(),
			"JSONL file should not exist until first message entry is persisted"
		);
	}

	#[test]
	fn test_delayed_write_creates_file_on_append() {
		let tmp = tempfile::tempdir().unwrap();
		let cwd = tmp.path().join("project");
		std::fs::create_dir_all(&cwd).unwrap();

		let mut mgr = SessionManager::create(&cwd, Some(tmp.path())).unwrap();
		let file_path = mgr.session_file().unwrap().to_path_buf();

		assert!(!file_path.exists(), "File should not exist before append");

		// Append a message entry — this should trigger file creation.
		mgr.append_message(Message::User(UserMessage { content: "Hello".to_owned() }))
			.unwrap();

		assert!(file_path.exists(), "JSONL file should exist after first message entry is appended");
	}

	#[test]
	fn test_persist_writes_valid_jsonl() {
		let tmp = tempfile::tempdir().unwrap();
		let cwd = tmp.path().join("project");
		std::fs::create_dir_all(&cwd).unwrap();

		let mut mgr = SessionManager::create(&cwd, Some(tmp.path())).unwrap();

		mgr.append_message(Message::User(UserMessage { content: "First message".to_owned() }))
			.unwrap();
		mgr.append_message(Message::User(UserMessage { content: "Second message".to_owned() }))
			.unwrap();

		let file_path = mgr.session_file().unwrap();
		let content = std::fs::read_to_string(file_path).unwrap();

		// Each non-empty line should be valid JSON.
		let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
		assert!(
			lines.len() >= 3,
			"Should have at least 3 lines (header + 2 entries), got {}",
			lines.len()
		);

		for (i, line) in lines.iter().enumerate() {
			let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
			assert!(parsed.is_ok(), "Line {i} should be valid JSON: {line}");
		}

		// First line should be the session header.
		let header: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
		assert_eq!(header["type"], "session");
		assert_eq!(header["version"], 1);

		// Remaining lines should be entries with type "message".
		for line in &lines[1..] {
			let entry: serde_json::Value = serde_json::from_str(line).unwrap();
			assert_eq!(entry["type"], "message");
		}
	}

	// -- Tolerant JSONL reader tests --

	/// Helper: build a valid session header JSON line.
	fn make_header_line(session_id: &str) -> String {
		serde_json::to_string(&serde_json::json!({
			"type": "session",
			"version": 1,
			"id": session_id,
			"timestamp": "2026-01-15T10:30:00+00:00",
			"cwd": "/tmp/test",
		}))
		.unwrap()
	}

	/// Helper: build a valid message entry JSON line.
	fn make_entry_line(id: &str, parent_id: Option<&str>, content: &str) -> String {
		serde_json::to_string(&serde_json::json!({
			"type": "message",
			"id": id,
			"parent_id": parent_id,
			"timestamp": "2026-01-15T10:31:00+00:00",
			"message": {
				"role": "user",
				"content": content,
			}
		}))
		.unwrap()
	}

	#[test]
	fn test_parse_jsonl_tolerates_malformed_last_line() {
		let header = make_header_line("0000000000000001");
		let entry1 = make_entry_line("aa000001", None, "Hello");
		let entry2 = make_entry_line("aa000002", Some("aa000001"), "World");
		let malformed = r#"{"type": "message", "id": "aa000003", BROKEN"#;

		let content = format!("{header}\n{entry1}\n{entry2}\n{malformed}\n");
		let (parsed_header, entries) = SessionManager::parse_jsonl(&content).unwrap();

		assert_eq!(parsed_header.id, "0000000000000001");
		// The two valid entries should be preserved; the malformed last line skipped.
		assert_eq!(entries.len(), 2);
	}

	#[test]
	fn test_parse_jsonl_fails_on_mid_file_corruption() {
		let header = make_header_line("0000000000000001");
		let entry1 = make_entry_line("aa000001", None, "Hello");
		let corrupted = "NOT VALID JSON AT ALL";
		let entry3 = make_entry_line("aa000003", Some("aa000001"), "After corruption");

		let content = format!("{header}\n{entry1}\n{corrupted}\n{entry3}\n");
		let result = SessionManager::parse_jsonl(&content);

		assert!(result.is_err(), "Mid-file corruption should cause an error");
	}

	#[test]
	fn test_parse_jsonl_valid_content_unchanged() {
		let header = make_header_line("0000000000000001");
		let entry1 = make_entry_line("aa000001", None, "Hello");
		let entry2 = make_entry_line("aa000002", Some("aa000001"), "World");

		let content = format!("{header}\n{entry1}\n{entry2}\n");
		let (_, entries) = SessionManager::parse_jsonl(&content).unwrap();

		assert_eq!(entries.len(), 2, "All valid entries should be parsed");
	}

	#[test]
	fn test_parse_jsonl_partial_write_without_newline() {
		// Simulates a crash mid-write where the last line has no trailing newline.
		let header = make_header_line("0000000000000001");
		let entry1 = make_entry_line("aa000001", None, "Hello");
		let partial = r#"{"type": "message", "id": "aa00"#; // truncated

		let content = format!("{header}\n{entry1}\n{partial}");
		let (_, entries) = SessionManager::parse_jsonl(&content).unwrap();

		assert_eq!(entries.len(), 1, "Only the valid entry should be parsed");
	}

	// -- Atomic FileSessionWriter tests --

	#[test]
	fn test_file_writer_large_entry_atomic() {
		let tmp = tempfile::tempdir().unwrap();
		let s = storage::FileSessionStorage;
		let p = tmp.path().join("large.jsonl");

		let mut writer = s.open_writer(&p, false).unwrap();

		// Write a line larger than the typical BufWriter buffer (8KB).
		let large_line = "x".repeat(16_000);
		writer.write_line(&large_line).unwrap();
		writer.flush().unwrap();
		writer.close().unwrap();

		let content = s.read_text(&p).unwrap();
		let lines: Vec<&str> = content.lines().collect();
		assert_eq!(lines.len(), 1, "Should have exactly one line");
		assert_eq!(lines[0], large_line, "Line content should match");
	}
}
