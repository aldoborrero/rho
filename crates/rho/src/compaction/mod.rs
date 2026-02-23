//! Context compaction: summarize older conversation turns to reclaim tokens.
//!
//! When a session grows close to the model's context window, compaction
//! generates an LLM summary of older messages, stores it as a
//! [`CompactionEntry`], and lets [`build_context()`] replay the compacted
//! state on the next agent turn.

pub mod compact;
pub mod cut_point;
pub mod file_ops;
pub mod serialize;
pub mod settings;
pub mod tokens;
