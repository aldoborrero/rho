//! Pure Rust utilities extracted from pi-natives, without N-API dependencies.
//!
//! Provides clipboard access, file discovery, grep, glob matching,
//! HTML-to-Markdown conversion, image processing, shell execution,
//! PTY management, process tree management, profiling, and workmux
//! integration.

#![allow(dead_code, reason = "Modules being extracted incrementally")]
#![allow(unused_imports, reason = "Modules being extracted incrementally")]

pub mod cancel;
pub mod clipboard;
pub mod error;
pub mod fd;
pub mod fs_cache;
pub mod glob;
pub mod glob_util;
pub mod grep;
pub mod html;
pub mod image;
pub mod prof;
pub mod ps;
pub mod pty;
pub mod shell;
pub mod workmux;

pub use cancel::CancelToken;
pub use error::{Error, Result};
