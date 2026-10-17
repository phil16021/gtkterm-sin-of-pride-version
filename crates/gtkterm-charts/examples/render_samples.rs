//! Render sample charts to PNG: `cargo run -p gtkterm-charts --example render_samples -- OUT_DIR`.
#![allow(clippy::unwrap_used)]
use gtkterm_charts::model::ViewState;
use gtkterm_charts::render::{Style, render_png};
use gtkterm_core::sunshine::SunshineStore;
use jiff::Timestamp;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut s = SunshineStore::new();
    for i in 0..300 {
        let t = f64::from(i) * 0.05;
        let lines = format!(
            "{{env|Temp|255,0,0|{:.3}|Humidity|0,0,255|{:.3}|Noise|{:.3}}}\n{{B:levels|Red|255,0,0|{:.1}|Green|0,200,0|{:.1}|Blue|0,0,255|{:.1}|Neg|-{:.1}}}\n{{P:share|CPU|{:.1}|IO|{:.1}|Idle|{:.1}}}\n{{S:orbit|path|{:.3} {:.3}|ref|{:.3} {:.3}}}\n{{mv|sig|{:.6}}}\n",
            21.0 + 3.0 * (t * 0.7).sin(),
            45.0 + 8.0 * (t * 0.3).cos(),
            (t * 13.0).sin(),
            50.0 + 40.0 * t.sin(),
            60.0,
            30.0,
            12.0,
            30.0,
            15.0,
            55.0,
            t.cos() * (1.0 + 0.2 * t.sin()),
            t.sin(),
            0.5 * t.cos(),
            0.5 * t.sin(),
            0.0015 + 0.0003 * (t * 2.0).sin()
        );
        s.feed(
            lines.as_bytes(),
            Timestamp::from_millisecond(i64::from(i) * 50).unwrap(),
        );
    }
    for (style, name) in [(Style::light(), "light"), (Style::dark(), "dark")] {
        for c in s.charts() {
            let png = render_png(c, &ViewState::default(), &style, 640, 380).unwrap();
            std::fs::write(format!("{out}/{}-{name}.png", c.id), png).unwrap();
        }
    }
}
