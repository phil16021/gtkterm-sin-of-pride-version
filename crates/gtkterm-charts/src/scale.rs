//! Axis ranges, "nice" tick steps and tick labels.
//!
//! The C charts forced any span below 0.01 (line) or 0.001 (bar/scatter) to 1.0 and printed
//! labels with `%.1f`, so millivolt signals became flat lines labelled `0.0` (ROADMAP S6).
//! Here the padding is relative to the data magnitude and labels follow the tick step.

/// A closed numeric interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    /// Lower bound.
    pub min: f64,
    /// Upper bound.
    pub max: f64,
}

impl Range {
    /// Construct, swapping bounds if needed.
    pub fn new(a: f64, b: f64) -> Self {
        if a <= b {
            Self { min: a, max: b }
        } else {
            Self { min: b, max: a }
        }
    }

    /// `max - min`.
    pub fn span(self) -> f64 {
        self.max - self.min
    }

    /// Map `v` to 0..1 within the range (0 for an empty range).
    pub fn normalize(self, v: f64) -> f64 {
        let span = self.span();
        if span > 0.0 { (v - self.min) / span } else { 0.0 }
    }

    /// Union with another range.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Smallest range containing all finite values, if any.
    pub fn of(values: impl IntoIterator<Item = f64>) -> Option<Self> {
        values
            .into_iter()
            .filter(|v| v.is_finite())
            .fold(None, |acc, v| match acc {
                None => Some(Self { min: v, max: v }),
                Some(r) => Some(Self {
                    min: r.min.min(v),
                    max: r.max.max(v),
                }),
            })
    }

    /// Widen by `fraction` of the span on both sides. A degenerate (constant) range gets a
    /// margin relative to its magnitude, or ±1 around zero.
    #[must_use]
    pub fn padded(self, fraction: f64) -> Self {
        let magnitude = self.min.abs().max(self.max.abs());
        let span = self.span();
        if span <= magnitude * 1e-9 || span == 0.0 {
            let half = if magnitude > 0.0 { magnitude * 0.1 } else { 1.0 };
            return Self {
                min: self.min - half,
                max: self.max + half,
            };
        }
        Self {
            min: self.min - span * fraction,
            max: self.max + span * fraction,
        }
    }

    /// Include zero (bar charts grow from a zero baseline).
    #[must_use]
    pub fn with_zero(self) -> Self {
        Self {
            min: self.min.min(0.0),
            max: self.max.max(0.0),
        }
    }
}

/// The nice number closest to `x` from {1, 2, 5} × 10^n (Heckbert).
fn nice(x: f64, round: bool) -> f64 {
    let exp = x.log10().floor();
    let f = x / 10f64.powf(exp);
    let nf = if round {
        if f < 1.5 {
            1.0
        } else if f < 3.0 {
            2.0
        } else if f < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    nf * 10f64.powf(exp)
}

/// Ticks for an axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticks {
    /// Distance between ticks.
    pub step: f64,
    /// Tick positions inside the range, ascending.
    pub values: Vec<f64>,
}

/// About `target` nicely spaced ticks within `range`.
pub fn ticks(range: Range, target: usize) -> Ticks {
    let target = target.max(2);
    let span = range.span();
    if !(span.is_finite() && span > 0.0) {
        return Ticks {
            step: 1.0,
            values: vec![range.min],
        };
    }
    let mut step = nice(nice(span, false) / (target - 1) as f64, true);
    loop {
        let values = tick_values(range, step);
        // Rounding the step up can leave fewer than two ticks inside a narrow range.
        if values.len() >= 2 || step <= span * 1e-6 {
            return Ticks { step, values };
        }
        step = nice(step / 2.0, true).min(step / 2.0);
    }
}

fn tick_values(range: Range, step: f64) -> Vec<f64> {
    let first = (range.min / step).ceil() * step;
    let mut values = Vec::new();
    let mut i = 0.0;
    loop {
        let v = first + i * step;
        if v > range.max + step * 1e-9 || values.len() > 1000 {
            break;
        }
        // Snap tiny float noise to zero so it prints as "0".
        values.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        i += 1.0;
    }
    values
}

/// Label for a tick value; decimals follow the step, very large/small numbers use exponents.
pub fn format_tick(v: f64, step: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if !(1e-4..1e7).contains(&a) {
        let digits = if step > 0.0 {
            ((a.log10() - step.log10()).floor().max(0.0) as usize).min(6)
        } else {
            2
        };
        return format!("{v:.digits$e}");
    }
    let decimals = if step > 0.0 {
        (-step.log10().floor()).max(0.0) as usize
    } else {
        2
    };
    format!("{v:.prec$}", prec = decimals.min(9))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn small_signals_are_not_flattened() {
        let r = Range::new(0.0012, 0.0018).padded(0.1);
        assert!(r.span() < 0.001, "{r:?}");
        let t = ticks(r, 5);
        assert!(t.step <= 0.0002, "{t:?}");
        let labels: Vec<_> = t.values.iter().map(|v| format_tick(*v, t.step)).collect();
        assert!(labels.contains(&"0.0014".to_owned()), "{labels:?}");
    }

    #[test]
    fn degenerate_ranges() {
        assert_eq!(Range::new(5.0, 5.0).padded(0.1), Range::new(4.5, 5.5));
        assert_eq!(Range::new(0.0, 0.0).padded(0.1), Range::new(-1.0, 1.0));
        assert_eq!(Range::new(-3.0, -3.0).padded(0.1), Range::new(-3.3, -2.7));
        assert_eq!(
            Range::of([f64::NAN, 2.0, -1.0, f64::INFINITY]),
            Some(Range::new(-1.0, 2.0))
        );
        assert_eq!(Range::of(Vec::<f64>::new()), None);
    }

    #[test]
    fn all_negative_and_zero_baseline() {
        let r = Range::new(-20.0, -5.0).with_zero();
        assert_eq!(r, Range::new(-20.0, 0.0));
        let t = ticks(r, 5);
        assert_eq!(t.values.last().copied(), Some(0.0));
        assert_eq!(format_tick(-15.0, t.step), "-15");
    }

    #[test]
    fn nice_steps() {
        assert_eq!(ticks(Range::new(0.0, 100.0), 6).step, 20.0);
        assert_eq!(ticks(Range::new(0.0, 1.0), 5).step, 0.2);
        assert_eq!(ticks(Range::new(3.0, 3.0), 5).values, vec![3.0]);
        assert_eq!(format_tick(1.0e9, 1.0e8), "1.0e9");
        assert_eq!(format_tick(0.25, 0.05), "0.25");
    }

    proptest! {
        #[test]
        fn ticks_are_inside_and_ascending(a in -1e6f64..1e6, b in -1e6f64..1e6, n in 2usize..12) {
            let r = Range::new(a, b).padded(0.05);
            let t = ticks(r, n);
            prop_assert!(!t.values.is_empty());
            prop_assert!(t.values.len() <= 4 * n + 2, "{} ticks for target {n}", t.values.len());
            prop_assert!(t.values.windows(2).all(|w| w[0] < w[1]));
            prop_assert!(t.values.iter().all(|v| *v >= r.min - t.step * 1e-6 && *v <= r.max + t.step * 1e-6));
        }
    }
}
