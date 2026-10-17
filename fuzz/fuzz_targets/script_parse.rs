//! The script parser never panics or hangs (includes resolve inside an empty directory).
#![no_main]
use std::sync::OnceLock;

use gtkterm_core::script::Script;
use libfuzzer_sys::fuzz_target;

fn sandbox() -> &'static std::path::Path {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("gtkterm-fuzz-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    })
}

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let dir = sandbox();
        let allowed = [dir.to_owned()];
        if let Ok(s) = Script::parse_with("fuzz", text, Some(dir), Some(&allowed)) {
            assert!(s.len() <= text.lines().count());
        }
    }
});
