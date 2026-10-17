//! The macro editor parser never panics, and formatting a parsed macro parses back to the same
//! commands. Imports of C-format macro files never panic either.
#![no_main]
use std::path::Path;

use gtkterm_core::macros::{format_text, import_c_macros, import_keyfile_macros, parse_text};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(commands) = parse_text(&text) {
        let again = parse_text(&format_text(&commands)).expect("formatted text parses");
        assert_eq!(again, commands);
    }
    let _ = import_c_macros(&text, Path::new("fuzz"));
    let _ = import_keyfile_macros(&text);
});
