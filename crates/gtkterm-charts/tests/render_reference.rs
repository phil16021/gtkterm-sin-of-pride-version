//! Rendered charts compared with reference images. Text rendering differs a little between
//! font setups, so a few percent of the pixels may differ; a broken axis, scale or colour
//! changes far more. `UPDATE_REFERENCE=1 cargo test -p gtkterm-charts --test render_reference`
//! rewrites the references (review them before committing).
#![cfg(feature = "render")]
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use gtkterm_charts::model::ViewState;
use gtkterm_charts::render::{Style, render_png};
use gtkterm_core::sunshine::SunshineStore;
use jiff::Timestamp;

/// Share of pixels that may differ noticeably.
const TOLERANCE: f64 = 0.03;
/// Per-channel difference that counts as noticeable.
const CHANNEL: u8 = 48;

fn store() -> SunshineStore {
    let mut s = SunshineStore::new();
    for i in 0..200 {
        let t = f64::from(i) * 0.05;
        let lines = format!(
            "{{env|Temp|255,0,0|{:.3}|Humidity|0,0,255|{:.3}}}\n{{B:levels|A|255,0,0|{:.1}|B|0,200,0|30|C|0,0,255|-12}}\n\
             {{P:share|CPU|50|IO|30|Idle|20}}\n{{S:orbit|path|{:.3} {:.3}}}\n",
            21.0 + 3.0 * (t * 0.7).sin(),
            45.0 + 8.0 * (t * 0.3).cos(),
            50.0 + 40.0 * t.sin(),
            t.cos() * (1.0 + 0.2 * t.sin()),
            t.sin(),
        );
        s.feed(
            lines.as_bytes(),
            Timestamp::from_millisecond(i64::from(i) * 50).unwrap(),
        );
    }
    s
}

fn pixels(png: &[u8]) -> (i32, i32, Vec<u8>) {
    let mut surface = cairo::ImageSurface::create_from_png(&mut std::io::Cursor::new(png)).unwrap();
    let (w, h, stride) = (surface.width(), surface.height(), surface.stride());
    let data = surface.data().unwrap();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let row = &data[(y * stride) as usize..(y * stride + w * 4) as usize];
        out.extend_from_slice(row);
    }
    (w, h, out)
}

/// Share of pixels with a channel differing by more than [`CHANNEL`].
fn difference(a: &[u8], b: &[u8]) -> f64 {
    let differing = a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > CHANNEL))
        .count();
    differing as f64 / (a.len() / 4) as f64
}

#[test]
fn charts_match_their_reference_images() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/reference");
    let update = std::env::var_os("UPDATE_REFERENCE").is_some();
    let s = store();
    let mut failures = Vec::new();
    for c in s.charts() {
        for (style, name) in [(Style::light(), "light"), (Style::dark(), "dark")] {
            let png = render_png(c, &ViewState::default(), &style, 480, 300).unwrap();
            let path = dir.join(format!("{}-{name}.png", c.id));
            if update || !path.exists() {
                std::fs::write(&path, &png).unwrap();
                continue;
            }
            let (w, h, got) = pixels(&png);
            let (rw, rh, want) = pixels(&std::fs::read(&path).unwrap());
            assert_eq!((w, h), (rw, rh), "{}: size", path.display());
            let d = difference(&got, &want);
            if d > TOLERANCE {
                let out = std::env::temp_dir().join(format!("{}-{name}.actual.png", c.id));
                std::fs::write(&out, &png).unwrap();
                failures.push(format!(
                    "{}: {:.1} % differ (actual: {})",
                    path.display(),
                    d * 100.0,
                    out.display()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_comparison_notices_a_different_chart() {
    let s = store();
    let env = s.chart("env").unwrap();
    let a = pixels(&render_png(env, &ViewState::default(), &Style::light(), 480, 300).unwrap()).2;
    let b = pixels(&render_png(env, &ViewState::default(), &Style::dark(), 480, 300).unwrap()).2;
    assert!(
        difference(&a, &b) > TOLERANCE,
        "light and dark must differ beyond the tolerance"
    );
    assert!(difference(&a, &a) < f64::EPSILON);
}
