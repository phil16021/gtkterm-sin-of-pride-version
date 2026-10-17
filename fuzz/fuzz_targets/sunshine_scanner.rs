//! Arbitrary bytes, fed in arbitrary splits, never panic the scanner/store and splitting does
//! not change the parsed packets.
#![no_main]
use gtkterm_core::sunshine::SunshineStore;
use gtkterm_protocols::sunshine::{ScanEvent, Scanner};
use libfuzzer_sys::fuzz_target;

fn packets(events: Vec<ScanEvent>) -> Vec<String> {
    events.into_iter().filter_map(|e| if let ScanEvent::Packet(p) = e { Some(format!("{p:?}")) } else { None }).collect()
}

fuzz_target!(|data: &[u8]| {
    let split = data.first().map_or(0, |&b| b as usize) % (data.len() + 1);
    let whole = packets(Scanner::new().feed(data));
    let mut s = Scanner::new();
    let mut parts = s.feed(&data[..split]);
    parts.extend(s.feed(&data[split..]));
    assert_eq!(packets(parts), whole);
    let mut store = SunshineStore::new();
    let _ = store.feed(data, jiff::Timestamp::UNIX_EPOCH);
    for c in store.charts() {
        let _ = c.to_csv();
    }
});
