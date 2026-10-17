//! The hex parser never panics and its output re-encodes to an equivalent input.
#![no_main]
use gtkterm_protocols::hex::{parse_hex, to_hex};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(bytes) = parse_hex(text) {
            assert_eq!(parse_hex(&to_hex(&bytes)), Ok(bytes));
        }
    }
});
