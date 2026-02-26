//! Slash command registry and dispatcher for interactive mode.
//!
//! All commands start with `/` and are handled locally without being sent to
//! the AI. The module provides [`parse_command`] to split raw input into a
//! command name and arguments, and [`execute_command`] to dispatch to the
//! appropriate handler.

mod dispatch;
pub mod handlers;
mod registry;
mod types;

pub use dispatch::{BangResult, execute_bang, execute_bang_streaming, execute_command};
pub use registry::{COMMANDS, parse_command};
pub use types::{CommandContext, CommandResult, SlashCommand, SubcommandDef};
