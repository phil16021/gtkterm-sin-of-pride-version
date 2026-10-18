//! Model Context Protocol server exposing GTKTerm sessions to AI agents.
//!
//! Protocol revision 2026-07-28 (stateless) is the default; rmcp negotiates down to
//! 2025-11-25 and older for clients that still use the initialize handshake.
#![forbid(unsafe_code)]

mod guard;
mod params;
mod server;
pub mod transport;

pub use guard::{Approval, ApprovalRequest, Approver};
pub use server::{
    GtkTermMcp, ServerOptions, WRITE_TOOLS, default_audit_path, default_log_dir, default_transfer_dir,
};

/// Reference of the automation script language (Markdown), also served as
/// `gtkterm://docs/script-language`.
pub const SCRIPT_LANGUAGE: &str = include_str!("script_language.md");

/// Reference of the Sunshine packet format (Markdown), also served as
/// `gtkterm://docs/sunshine-protocol`.
pub const SUNSHINE_PROTOCOL: &str = include_str!("sunshine_protocol.md");
