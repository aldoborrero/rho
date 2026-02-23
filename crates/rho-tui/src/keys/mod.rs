//! Kitty keyboard protocol parsing and key matching.
//!
//! Parses terminal keyboard input including:
//! - Kitty keyboard protocol (CSI-u, functional keys, CSI 1;mod letter)
//! - Legacy escape sequences (arrow keys, function keys, xterm)
//! - xterm modifyOtherKeys format
//! - Single-byte ASCII/control characters
//!
//! # Example
//! ```
//! use rho_tui::keys::{matches_key, parse_key};
//!
//! assert_eq!(parse_key(b"\x1b", false), Some("escape".into()));
//! assert!(matches_key(b"\x03", "ctrl+c", false));
//! ```

pub mod legacy;
pub mod match_key;
pub mod parse;
pub mod types;

pub use match_key::matches_key;
pub use parse::{parse_key, parse_kitty_sequence};
pub use types::{ParsedKittyResult, ParsedKittySequence};
