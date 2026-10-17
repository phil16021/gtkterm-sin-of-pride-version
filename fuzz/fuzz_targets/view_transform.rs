//! View formatter and CR/LF transform accept any bytes in any mode.
#![no_main]
use gtkterm_protocols::transform::{Transform, TransformOptions};
use gtkterm_protocols::view::{ViewFormatter, ViewMode, ViewSettings};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&ctl, rest)) = data.split_first() else { return };
    let mode = ViewMode::ALL[ctl as usize % ViewMode::ALL.len()];
    let settings = ViewSettings { mode, show_index: ctl & 0x80 != 0, hex_chars: 1 + (ctl as usize % 64), columns: 1 + ctl as usize };
    let mut f = ViewFormatter::new(settings);
    let out = f.format(rest);
    if mode != ViewMode::Ascii {
        assert!(out.is_ascii());
    }
    let opts = TransformOptions { auto_cr: ctl & 1 != 0, auto_lf: ctl & 2 != 0, timestamps: ctl & 4 != 0, ..Default::default() };
    let mut t = Transform::new(opts, jiff::tz::TimeZone::UTC, jiff::Timestamp::UNIX_EPOCH);
    let _ = t.apply(rest, jiff::Timestamp::UNIX_EPOCH);
});
