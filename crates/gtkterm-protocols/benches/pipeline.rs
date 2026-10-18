//! Receive pipeline costs: the view formatter (every dump mode) and the Sunshine scanner.
//! `cargo bench -p gtkterm-protocols`.
#![allow(missing_docs, clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use gtkterm_protocols::sunshine::Scanner;
use gtkterm_protocols::view::{ViewFormatter, ViewMode, ViewSettings};

fn data(len: usize) -> Vec<u8> {
    b"{imu|Roll|231,76,60|19.59|Pitch|46,204,113|-4.43|Yaw|52,152,219|30.34}\r\n[ 11.950] heap free 177 kB\r\n"
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

fn formatter(c: &mut Criterion) {
    let input = data(1 << 20);
    let mut g = c.benchmark_group("view_formatter");
    g.throughput(Throughput::Bytes(input.len() as u64));
    for mode in [ViewMode::Ascii, ViewMode::Hex, ViewMode::Mixed, ViewMode::Binary] {
        g.bench_function(mode.label(), |b| {
            b.iter(|| {
                let mut f = ViewFormatter::new(ViewSettings {
                    mode,
                    show_index: true,
                    hex_chars: 16,
                    columns: 120,
                });
                black_box(f.format_stamped(black_box(&input), Some("12:00:00.000")))
            });
        });
    }
    g.finish();
}

fn scanner(c: &mut Criterion) {
    let input = data(1 << 20);
    let mut g = c.benchmark_group("sunshine_scanner");
    g.throughput(Throughput::Bytes(input.len() as u64));
    g.bench_function("1 MiB in 4 KiB reads", |b| {
        b.iter(|| {
            let mut s = Scanner::new();
            let mut n = 0;
            for chunk in input.chunks(4096) {
                n += s.feed(black_box(chunk)).len();
            }
            black_box(n)
        });
    });
    g.finish();
}

criterion_group!(benches, formatter, scanner);
criterion_main!(benches);
