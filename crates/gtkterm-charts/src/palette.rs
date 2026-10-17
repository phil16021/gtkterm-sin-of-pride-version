//! Series colours.

use gtkterm_protocols::sunshine::Rgb;

/// RGB in 0.0–1.0.
pub type Color = (f64, f64, f64);

/// Distinct default colours (Tableau 10), used when a packet gives no colour. The C charts drew
/// every uncoloured series in the same blue (ROADMAP S6).
const PALETTE: [Color; 10] = [
    (0.122, 0.467, 0.706),
    (1.000, 0.498, 0.055),
    (0.173, 0.627, 0.173),
    (0.839, 0.153, 0.157),
    (0.580, 0.404, 0.741),
    (0.549, 0.337, 0.294),
    (0.890, 0.467, 0.761),
    (0.498, 0.498, 0.498),
    (0.737, 0.741, 0.133),
    (0.090, 0.745, 0.812),
];

/// Colour for the series at `index`, preferring the packet's colour.
pub fn series_color(index: usize, packet: Option<Rgb>) -> Color {
    match packet {
        Some(c) => (
            f64::from(c.r) / 255.0,
            f64::from(c.g) / 255.0,
            f64::from(c.b) / 255.0,
        ),
        None => PALETTE[index % PALETTE.len()],
    }
}

/// HSV (h in 0..1, s, v in 0..1) to RGB.
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> Color {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_colour_wins_and_defaults_differ() {
        assert_eq!(series_color(3, Some(Rgb { r: 255, g: 0, b: 0 })), (1.0, 0.0, 0.0));
        assert_ne!(series_color(0, None), series_color(1, None));
        assert_eq!(series_color(0, None), series_color(10, None));
    }

    #[test]
    fn hsv() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (1.0, 0.0, 0.0));
        let (r, g, b) = hsv_to_rgb(1.0 / 3.0, 1.0, 1.0);
        assert!(r.abs() < 1e-9 && (g - 1.0).abs() < 1e-9 && b.abs() < 1e-9);
        assert_eq!(hsv_to_rgb(0.5, 0.0, 0.5), (0.5, 0.5, 0.5));
    }
}
