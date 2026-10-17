//! Chunk store caps and cursor reads hold for arbitrary push/read sequences.
#![no_main]
use bytes::Bytes;
use gtkterm_core::chunks::{ChunkStore, Direction};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut s = ChunkStore::new(1 + data.len() % 300, 1 + data.len() % 17);
    let mut cursor = 0u64;
    for (i, chunk) in data.chunks(7).enumerate() {
        let dir = if chunk[0] & 1 == 0 { Direction::Rx } else { Direction::Tx };
        s.push(dir, Bytes::copy_from_slice(chunk), jiff::Timestamp::UNIX_EPOCH);
        assert!(s.len() == 1 || s.bytes() <= 1 + data.len() % 300);
        if i % 3 == 0 {
            let r = s.read_rx(cursor, chunk.len());
            assert!(r.next_cursor >= cursor);
            assert!(r.next_cursor <= s.rx_total());
            cursor = r.next_cursor;
        }
    }
});
