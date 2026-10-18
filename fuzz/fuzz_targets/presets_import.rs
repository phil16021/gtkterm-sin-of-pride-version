//! C preset files never panic the importer; what it accepts exports and imports again to the
//! same presets, and applying them gives a valid serial configuration.
#![no_main]
use gtkterm_core::config::{Connection, SerialConfig};
use gtkterm_core::presets::{export_c_presets, import_c_presets};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let Ok(presets) = import_c_presets(&text) else { return };
    let again = import_c_presets(&export_c_presets(&presets)).expect("exported presets import");
    assert_eq!(again, presets);
    for (_, p) in &presets {
        let mut cfg = SerialConfig {
            port: "/dev/ttyS0".into(),
            ..SerialConfig::default()
        };
        p.apply(&mut cfg);
        let _ = p.summary();
        let _ = Connection::Serial(cfg).validate();
    }
});
