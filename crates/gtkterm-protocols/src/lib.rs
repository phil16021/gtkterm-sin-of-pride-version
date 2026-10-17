//! Sans-IO protocol logic for GTKTerm Sunshine.
//!
//! Everything in this crate is pure: no I/O, no clocks (timestamps are passed in),
//! no global state. That keeps it unit-testable, fuzzable and usable from both the
//! GUI and the headless MCP server.
#![forbid(unsafe_code)]

pub mod checksum;
pub mod decode;
pub mod downsample;
pub mod escape;
pub mod hex;
pub mod logfmt;
pub mod sunshine;
pub mod transform;
pub mod view;
