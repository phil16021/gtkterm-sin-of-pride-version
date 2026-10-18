//! History costs: appending received chunks (with eviction) and reading by cursor.
//! `cargo bench -p gtkterm-core`.
#![allow(missing_docs, clippy::unwrap_used)]

use std::hint::black_box;

use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use gtkterm_core::chunks::{ChunkStore, Direction};
use jiff::Timestamp;

fn store(c: &mut Criterion) {
    let chunk = Bytes::from(vec![b'x'; 256]);
    let ts = Timestamp::UNIX_EPOCH;
    let mut g = c.benchmark_group("chunk_store");
    g.throughput(Throughput::Bytes(256 * 10_000));
    g.bench_function("push 10k chunks (evicting)", |b| {
        b.iter(|| {
            let mut s = ChunkStore::new(1 << 20, 4096);
            for _ in 0..10_000 {
                black_box(s.push(Direction::Rx, chunk.clone(), ts));
            }
        });
    });
    let mut full = ChunkStore::new(64 << 20, 1 << 20);
    for _ in 0..100_000 {
        full.push(Direction::Rx, chunk.clone(), ts);
    }
    g.throughput(Throughput::Bytes(64 * 1024));
    g.bench_function("read 64 KiB at a cursor in 100k chunks", |b| {
        let mut cursor = 0u64;
        b.iter(|| {
            cursor = (cursor + 1_234_567) % (25 << 20);
            black_box(full.read_rx(black_box(cursor), 64 * 1024))
        });
    });
    g.finish();
}

criterion_group!(benches, store);
criterion_main!(benches);
