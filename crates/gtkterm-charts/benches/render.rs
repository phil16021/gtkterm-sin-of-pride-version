//! Chart rendering cost at the default history (10k points per series).
//! `cargo bench -p gtkterm-charts`.
#![allow(missing_docs, clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gtkterm_charts::model::ViewState;
use gtkterm_charts::render::{Style, draw};
use gtkterm_core::sunshine::SunshineStore;
use jiff::Timestamp;

fn render(c: &mut Criterion) {
    let mut s = SunshineStore::new();
    s.feed(b"{imu|Roll|0|Pitch|0|Yaw|0}\n", Timestamp::UNIX_EPOCH);
    s.chart_mut("imu").unwrap().set_capacity(10_000);
    for i in 0..10_000 {
        let t = f64::from(i) * 0.01;
        let line = format!(
            "{{imu|Roll|{:.3}|Pitch|{:.3}|Yaw|{:.3}}}\n",
            t.sin() * 25.0,
            (t * 2.0).cos() * 15.0,
            t * 0.1
        );
        s.feed(
            line.as_bytes(),
            Timestamp::from_millisecond(i64::from(i) * 10).unwrap(),
        );
    }
    let chart = s.chart("imu").unwrap().clone();
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1200, 700).unwrap();
    let style = Style::dark();
    c.bench_function("line chart, 3 series × 10k points, 1200×700", |b| {
        b.iter(|| {
            let cr = cairo::Context::new(&surface).unwrap();
            draw(
                &cr,
                black_box(&chart),
                &ViewState::default(),
                &style,
                1200.0,
                700.0,
                None,
            )
            .unwrap();
        });
    });
}

criterion_group!(benches, render);
criterion_main!(benches);
