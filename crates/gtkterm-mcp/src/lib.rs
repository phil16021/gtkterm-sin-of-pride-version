//! Model Context Protocol server exposing GTKTerm sessions to AI agents.
//!
//! Protocol revision 2026-07-28 (stateless) is the default; rmcp negotiates down to
//! 2025-11-25 and older for clients that still use the initialize handshake.
#![forbid(unsafe_code)]

mod params;
mod server;
pub mod transport;

pub use server::{GtkTermMcp, ServerOptions, WRITE_TOOLS, default_log_dir, default_transfer_dir};
