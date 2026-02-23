//! Session entry types for JSONL session persistence.
//!
//! All types are serde-compatible with the TypeScript session format,
//! using camelCase JSON field names and a `"type"` discriminator tag.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::types::Message;

// ---------------------------------------------------------------------------
// Session header (first line of JSONL file)
// ---------------------------------------------------------------------------

/// The first line of a session JSONL file, containing session metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHeader {
	/// Always `"session"`.
	pub r#type:         String,
	/// Schema version (currently 1).
	pub version:        u32,
	/// Snowflake session ID (16-char hex).
	pub id:             String,
	/// Optional human-readable title.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title:          Option<String>,
	/// ISO 8601 timestamp of session creation.
	pub timestamp:      String,
	/// Working directory when the session was created.
	pub cwd:            String,
	/// Snowflake ID of the parent session, if this is a fork.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_session: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry structs (11 variants)
// ---------------------------------------------------------------------------

/// A chat message entry (user, assistant, or tool result).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageEntry {
	pub id:        String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id: Option<String>,
	pub timestamp: String,
	pub message:   Message,
}

/// Records a change in the thinking/reasoning level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingLevelChangeEntry {
	pub id:             String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:      Option<String>,
	pub timestamp:      String,
	pub thinking_level: String,
}

/// Records a change to the model being used.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChangeEntry {
	pub id:        String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id: Option<String>,
	pub timestamp: String,
	/// Model identifier in `"provider/modelId"` format.
	pub model:     String,
	/// Role hint: `"default"`, `"smol"`, or `"slow"`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub role:      Option<String>,
}

/// A compaction event that summarises older entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionEntry {
	pub id:                  String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:           Option<String>,
	pub timestamp:           String,
	pub summary:             String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub short_summary:       Option<String>,
	pub first_kept_entry_id: String,
	pub tokens_before:       u64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub details:             Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub preserve_data:       Option<serde_json::Map<String, serde_json::Value>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub from_extension:      Option<bool>,
}

/// A summary of a branch point in the conversation tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryEntry {
	pub id:             String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:      Option<String>,
	pub timestamp:      String,
	pub from_id:        String,
	pub summary:        String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub details:        Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub from_extension: Option<bool>,
}

/// An extension-defined custom entry with opaque data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEntry {
	pub id:          String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:   Option<String>,
	pub timestamp:   String,
	pub custom_type: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub data:        Option<serde_json::Value>,
}

/// A custom message entry that can be displayed in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessageEntry {
	pub id:          String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:   Option<String>,
	pub timestamp:   String,
	pub custom_type: String,
	pub content:     String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub details:     Option<serde_json::Value>,
	pub display:     bool,
}

/// A label applied to another entry (e.g. for bookmarking).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelEntry {
	pub id:        String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id: Option<String>,
	pub timestamp: String,
	pub target_id: String,
	pub label:     Option<String>,
}

/// Injection of TTSR (tool-type-specific rules) into the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsrInjectionEntry {
	pub id:             String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:      Option<String>,
	pub timestamp:      String,
	pub injected_rules: Vec<String>,
}

/// Initial session setup entry with system prompt and task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInitEntry {
	pub id:            String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id:     Option<String>,
	pub timestamp:     String,
	pub system_prompt: String,
	pub task:          String,
	pub tools:         Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_schema: Option<serde_json::Value>,
}

/// Records a change to the agent operating mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeChangeEntry {
	pub id:        String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parent_id: Option<String>,
	pub timestamp: String,
	pub mode:      String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub data:      Option<serde_json::Map<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// SessionEntry enum (internally tagged by "type")
// ---------------------------------------------------------------------------

/// Discriminated union of all session entry types, tagged by `"type"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
	Message(SessionMessageEntry),
	ThinkingLevelChange(ThinkingLevelChangeEntry),
	ModelChange(ModelChangeEntry),
	Compaction(CompactionEntry),
	BranchSummary(BranchSummaryEntry),
	Custom(CustomEntry),
	CustomMessage(CustomMessageEntry),
	Label(LabelEntry),
	TtsrInjection(TtsrInjectionEntry),
	SessionInit(SessionInitEntry),
	ModeChange(ModeChangeEntry),
}

impl SessionEntry {
	/// Return the entry's unique ID (8-char hex).
	pub fn id(&self) -> &str {
		match self {
			Self::Message(e) => &e.id,
			Self::ThinkingLevelChange(e) => &e.id,
			Self::ModelChange(e) => &e.id,
			Self::Compaction(e) => &e.id,
			Self::BranchSummary(e) => &e.id,
			Self::Custom(e) => &e.id,
			Self::CustomMessage(e) => &e.id,
			Self::Label(e) => &e.id,
			Self::TtsrInjection(e) => &e.id,
			Self::SessionInit(e) => &e.id,
			Self::ModeChange(e) => &e.id,
		}
	}

	/// Return the optional parent entry ID.
	pub fn parent_id(&self) -> Option<&str> {
		match self {
			Self::Message(e) => e.parent_id.as_deref(),
			Self::ThinkingLevelChange(e) => e.parent_id.as_deref(),
			Self::ModelChange(e) => e.parent_id.as_deref(),
			Self::Compaction(e) => e.parent_id.as_deref(),
			Self::BranchSummary(e) => e.parent_id.as_deref(),
			Self::Custom(e) => e.parent_id.as_deref(),
			Self::CustomMessage(e) => e.parent_id.as_deref(),
			Self::Label(e) => e.parent_id.as_deref(),
			Self::TtsrInjection(e) => e.parent_id.as_deref(),
			Self::SessionInit(e) => e.parent_id.as_deref(),
			Self::ModeChange(e) => e.parent_id.as_deref(),
		}
	}

	/// Return the ISO 8601 timestamp of the entry.
	pub fn timestamp(&self) -> &str {
		match self {
			Self::Message(e) => &e.timestamp,
			Self::ThinkingLevelChange(e) => &e.timestamp,
			Self::ModelChange(e) => &e.timestamp,
			Self::Compaction(e) => &e.timestamp,
			Self::BranchSummary(e) => &e.timestamp,
			Self::Custom(e) => &e.timestamp,
			Self::CustomMessage(e) => &e.timestamp,
			Self::Label(e) => &e.timestamp,
			Self::TtsrInjection(e) => &e.timestamp,
			Self::SessionInit(e) => &e.timestamp,
			Self::ModeChange(e) => &e.timestamp,
		}
	}
}

// ---------------------------------------------------------------------------
// FileEntry — for parsing JSONL lines (header OR entry)
// ---------------------------------------------------------------------------

/// A single line in the session JSONL file: either the header or an entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FileEntry {
	Header(SessionHeader),
	Entry(SessionEntry),
}

// ---------------------------------------------------------------------------
// Query / result types
// ---------------------------------------------------------------------------

/// Reconstructed conversation context from a session file.
pub struct SessionContext {
	pub messages:            Vec<Message>,
	pub thinking_level:      String,
	pub models:              HashMap<String, String>,
	pub injected_ttsr_rules: Vec<String>,
	pub mode:                String,
	pub mode_data:           Option<serde_json::Map<String, serde_json::Value>>,
}

/// Summary information about a persisted session.
pub struct SessionInfo {
	pub path:                PathBuf,
	pub id:                  String,
	pub cwd:                 String,
	pub title:               Option<String>,
	pub parent_session_path: Option<String>,
	pub created:             chrono::DateTime<chrono::Utc>,
	pub modified:            chrono::DateTime<chrono::Utc>,
	pub message_count:       usize,
	pub first_message:       String,
}

/// A node in the conversation tree with children and an optional label.
pub struct SessionTreeNode {
	pub entry:    SessionEntry,
	pub children: Vec<Self>,
	pub label:    Option<String>,
}

/// Result of a session fork operation.
pub struct ForkResult {
	pub old_session_file: Option<PathBuf>,
	pub new_session_file: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ai::types::UserMessage;

	fn make_timestamp() -> String {
		"2025-01-15T10:30:00Z".to_owned()
	}

	#[test]
	fn test_session_header_roundtrip() {
		let header = SessionHeader {
			r#type:         "session".to_owned(),
			version:        1,
			id:             "0123456789abcdef".to_owned(),
			title:          Some("Test Session".to_owned()),
			timestamp:      make_timestamp(),
			cwd:            "/home/user/project".to_owned(),
			parent_session: None,
		};

		let json = serde_json::to_string(&header).unwrap();
		let parsed: SessionHeader = serde_json::from_str(&json).unwrap();

		assert_eq!(parsed.r#type, "session");
		assert_eq!(parsed.version, 1);
		assert_eq!(parsed.id, "0123456789abcdef");
		assert_eq!(parsed.title.as_deref(), Some("Test Session"));
		assert_eq!(parsed.cwd, "/home/user/project");
		assert!(parsed.parent_session.is_none());
	}

	#[test]
	fn test_message_entry_roundtrip() {
		let entry = SessionMessageEntry {
			id:        "aabbccdd".to_owned(),
			parent_id: Some("11223344".to_owned()),
			timestamp: make_timestamp(),
			message:   Message::User(UserMessage { content: "Hello, world!".to_owned() }),
		};

		let json = serde_json::to_string(&entry).unwrap();
		let parsed: SessionMessageEntry = serde_json::from_str(&json).unwrap();

		assert_eq!(parsed.id, "aabbccdd");
		assert_eq!(parsed.parent_id.as_deref(), Some("11223344"));
		match &parsed.message {
			Message::User(u) => assert_eq!(u.content, "Hello, world!"),
			_ => panic!("Expected User message"),
		}
	}

	#[test]
	fn test_compaction_entry_roundtrip() {
		let entry = CompactionEntry {
			id:                  "aabbccdd".to_owned(),
			parent_id:           Some("11223344".to_owned()),
			timestamp:           make_timestamp(),
			summary:             "Discussed project setup".to_owned(),
			short_summary:       Some("Setup".to_owned()),
			first_kept_entry_id: "55667788".to_owned(),
			tokens_before:       50_000,
			details:             Some(serde_json::json!({"key": "value"})),
			preserve_data:       Some({
				let mut map = serde_json::Map::new();
				map.insert("ctx".to_owned(), serde_json::json!("preserved"));
				map
			}),
			from_extension:      Some(false),
		};

		let json = serde_json::to_string(&entry).unwrap();
		let parsed: CompactionEntry = serde_json::from_str(&json).unwrap();

		assert_eq!(parsed.summary, "Discussed project setup");
		assert_eq!(parsed.short_summary.as_deref(), Some("Setup"));
		assert_eq!(parsed.first_kept_entry_id, "55667788");
		assert_eq!(parsed.tokens_before, 50_000);
		assert!(parsed.details.is_some());
		assert!(parsed.preserve_data.is_some());
		assert_eq!(parsed.from_extension, Some(false));
	}

	#[test]
	fn test_entry_enum_tag_serialization() {
		let entry = SessionEntry::Message(SessionMessageEntry {
			id:        "aabbccdd".to_owned(),
			parent_id: None,
			timestamp: make_timestamp(),
			message:   Message::User(UserMessage { content: "test".to_owned() }),
		});

		let json = serde_json::to_string(&entry).unwrap();
		let value: serde_json::Value = serde_json::from_str(&json).unwrap();

		assert_eq!(value["type"], "message", "enum tag should be \"message\"");
	}

	#[test]
	fn test_file_entry_untagged_header() {
		let header_json = serde_json::json!({
			 "type": "session",
			 "version": 1,
			 "id": "0123456789abcdef",
			 "timestamp": "2025-01-15T10:30:00Z",
			 "cwd": "/tmp"
		});

		let parsed: FileEntry = serde_json::from_value(header_json).unwrap();
		match parsed {
			FileEntry::Header(h) => {
				assert_eq!(h.r#type, "session");
				assert_eq!(h.version, 1);
			},
			FileEntry::Entry(_) => panic!("Expected Header, got Entry"),
		}
	}

	#[test]
	fn test_file_entry_untagged_entry() {
		let entry_json = serde_json::json!({
			 "type": "model_change",
			 "id": "aabbccdd",
			 "timestamp": "2025-01-15T10:30:00Z",
			 "model": "anthropic/claude-sonnet-4-20250514"
		});

		let parsed: FileEntry = serde_json::from_value(entry_json).unwrap();
		match parsed {
			FileEntry::Entry(SessionEntry::ModelChange(mc)) => {
				assert_eq!(mc.model, "anthropic/claude-sonnet-4-20250514");
			},
			FileEntry::Entry(_) => panic!("Expected ModelChange entry"),
			FileEntry::Header(_) => panic!("Expected Entry, got Header"),
		}
	}

	#[test]
	fn test_entry_id_accessor() {
		let entry = SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
			id:             "deadbeef".to_owned(),
			parent_id:      None,
			timestamp:      make_timestamp(),
			thinking_level: "high".to_owned(),
		});

		assert_eq!(entry.id(), "deadbeef");
	}

	#[test]
	fn test_entry_parent_id_accessor() {
		let with_parent = SessionEntry::Label(LabelEntry {
			id:        "aabb0011".to_owned(),
			parent_id: Some("ccdd2233".to_owned()),
			timestamp: make_timestamp(),
			target_id: "eeff4455".to_owned(),
			label:     Some("bookmark".to_owned()),
		});
		assert_eq!(with_parent.parent_id(), Some("ccdd2233"));

		let without_parent = SessionEntry::Label(LabelEntry {
			id:        "aabb0011".to_owned(),
			parent_id: None,
			timestamp: make_timestamp(),
			target_id: "eeff4455".to_owned(),
			label:     None,
		});
		assert_eq!(without_parent.parent_id(), None);
	}
}
