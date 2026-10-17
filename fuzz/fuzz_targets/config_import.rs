//! Legacy key files and TOML configs never panic the loaders; imported configs validate and
//! survive a TOML round trip.
#![no_main]
use std::path::Path;

use gtkterm_core::config::{Config, import_legacy_keyfile};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (cfg, _warnings) = import_legacy_keyfile(&text);
    for p in cfg.profiles.values() {
        assert!(p.validate().is_ok());
    }
    let toml = cfg.to_toml().expect("serializable");
    assert_eq!(Config::from_toml(&toml, Path::new("fuzz")).expect("round trip"), cfg);
    let _ = Config::from_toml(&text, Path::new("fuzz"));
});
