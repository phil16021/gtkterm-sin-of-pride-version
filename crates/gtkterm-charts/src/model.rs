//! Plot models: what to draw for a chart, independent of the drawing backend.
//!
//! Sample mapping (same as the C charts): a sample with one value is a Y value plotted against
//! the sample index (or time); a sample with two or more values is a list of X/Y pairs.

use gtkterm_core::sunshine::{ChartData, Sample, SeriesData};
use gtkterm_protocols::downsample::lttb_indices;
use gtkterm_protocols::sunshine::ChartKind;

use crate::palette::{Color, series_color};
use crate::scale::Range;

/// What the X axis of line charts shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XAxis {
    /// Per-series sample number.
    #[default]
    Index,
    /// Seconds since the chart's first packet.
    Time,
}

/// User view state of one chart (zoom, pan, axis choice).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    /// X axis for single-value series.
    pub x_axis: XAxis,
    /// Visible X window; `None` follows the data (shows everything retained).
    pub x_window: Option<Range>,
    /// Points per series after downsampling (roughly 2× the plot width in pixels).
    pub max_points: usize,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            x_axis: XAxis::Index,
            x_window: None,
            max_points: 2000,
        }
    }
}

impl ViewState {
    /// Zoom the X window around `center` by `factor` (< 1 zooms in), starting from `full`.
    pub fn zoom(&mut self, full: Range, center: f64, factor: f64) {
        let cur = self.x_window.unwrap_or(full);
        let factor = factor.clamp(0.05, 20.0);
        let span = (cur.span() * factor).max(full.span() * 1e-6).max(1e-9);
        let rel = cur.normalize(center).clamp(0.0, 1.0);
        let min = center - span * rel;
        let w = Range::new(min, min + span);
        self.x_window = if w.min <= full.min && w.max >= full.max {
            None
        } else {
            Some(w)
        };
    }

    /// Shift the X window by `delta` data units (positive moves right).
    pub fn pan(&mut self, full: Range, delta: f64) {
        let cur = self.x_window.unwrap_or(full);
        self.x_window = Some(Range::new(cur.min + delta, cur.max + delta));
    }

    /// Back to following the data.
    pub fn reset(&mut self) {
        self.x_window = None;
    }
}

/// One series ready to plot.
#[derive(Debug, Clone, PartialEq)]
pub struct PlotSeries {
    /// Index in the chart's series list.
    pub index: usize,
    /// Name.
    pub name: String,
    /// Colour.
    pub color: Color,
    /// Shown or toggled off in the legend.
    pub visible: bool,
    /// Points in data coordinates, sorted by X for index/time data.
    pub points: Vec<(f64, f64)>,
}

/// Line/scatter plot model.
#[derive(Debug, Clone, PartialEq)]
pub struct XyModel {
    /// Series.
    pub series: Vec<PlotSeries>,
    /// X range shown.
    pub x: Range,
    /// Y range shown (padded).
    pub y: Range,
    /// Full X extent of the retained data (for zoom limits).
    pub x_full: Range,
    /// True if any series uses X/Y pairs.
    pub pairs: bool,
}

fn sample_points(s: &Sample, axis: XAxis, first_ms: i64) -> Vec<(f64, f64)> {
    if s.values.len() >= 2 {
        s.values.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect()
    } else {
        let x = match axis {
            XAxis::Index => s.index as f64,
            XAxis::Time => (s.ts.as_millisecond() - first_ms) as f64 / 1000.0,
        };
        s.values.first().map(|&y| vec![(x, y)]).unwrap_or_default()
    }
}

fn raw_points(series: &SeriesData, axis: XAxis, first_ms: i64) -> (Vec<(f64, f64)>, bool) {
    let pairs = series.samples.iter().any(|s| s.values.len() >= 2);
    let points = series
        .samples
        .iter()
        .flat_map(|s| sample_points(s, axis, first_ms))
        .collect();
    (points, pairs)
}

/// Build the model for line and scatter charts.
pub fn xy_model(chart: &ChartData, view: &ViewState) -> XyModel {
    let first_ms = chart.first_ts.as_millisecond();
    let mut all: Vec<(PlotSeries, bool)> = chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (points, pairs) = raw_points(s, view.x_axis, first_ms);
            (
                PlotSeries {
                    index: i,
                    name: s.name.clone(),
                    color: series_color(i, s.color),
                    visible: s.visible,
                    points,
                },
                pairs,
            )
        })
        .collect();
    let pairs = all.iter().any(|(_, p)| *p);
    let x_full = Range::of(
        all.iter()
            .filter(|(s, _)| s.visible)
            .flat_map(|(s, _)| s.points.iter().map(|p| p.0)),
    )
    .unwrap_or(Range::new(0.0, 1.0));
    let x = view.x_window.unwrap_or(x_full);

    for (s, is_pairs) in &mut all {
        if view.x_window.is_some() {
            if *is_pairs {
                s.points.retain(|p| p.0 >= x.min && p.0 <= x.max);
            } else {
                // Keep one point beyond each edge so lines reach the plot border.
                let start = s.points.partition_point(|p| p.0 < x.min).saturating_sub(1);
                let end = (s.points.partition_point(|p| p.0 <= x.max) + 1).min(s.points.len());
                s.points = s.points[start..end.max(start)].to_vec();
            }
        }
        if !*is_pairs && s.points.len() > view.max_points {
            let keep = lttb_indices(&s.points, view.max_points);
            s.points = keep.into_iter().map(|i| s.points[i]).collect();
        }
    }
    let y = Range::of(all.iter().filter(|(s, _)| s.visible).flat_map(|(s, _)| {
        s.points
            .iter()
            .filter(|p| p.0 >= x.min && p.0 <= x.max)
            .map(|p| p.1)
    }))
    .unwrap_or(Range::new(0.0, 1.0))
    .padded(0.1);
    let x = if view.x_window.is_some() {
        x
    } else {
        x.padded(if pairs { 0.1 } else { 0.0 })
    };
    XyModel {
        series: all.into_iter().map(|(s, _)| s).collect(),
        x,
        y,
        x_full,
        pairs,
    }
}

/// One bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    /// Index in the chart's series list.
    pub index: usize,
    /// Series name (category label).
    pub name: String,
    /// Colour.
    pub color: Color,
    /// Latest value.
    pub value: f64,
    /// Shown or toggled off.
    pub visible: bool,
}

/// Latest value of each series (first value of the last sample).
pub fn bars(chart: &ChartData) -> Vec<Bar> {
    chart
        .series
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let value = s.last()?.values.first().copied()?;
            Some(Bar {
                index: i,
                name: s.name.clone(),
                color: series_color(i, s.color),
                value,
                visible: s.visible,
            })
        })
        .collect()
}

/// Value range of a bar chart: always contains zero (bars grow from the baseline).
pub fn bar_range(bars: &[Bar]) -> Range {
    Range::of(bars.iter().filter(|b| b.visible).map(|b| b.value))
        .unwrap_or(Range::new(0.0, 1.0))
        .with_zero()
        .padded(0.08)
        .with_zero()
}

/// One pie slice.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    /// Index in the chart's series list (also the legend entry).
    pub index: usize,
    /// Series name.
    pub name: String,
    /// Colour — the same in the pie and in the legend.
    pub color: Color,
    /// Latest value (non-positive values get no slice).
    pub value: f64,
    /// Share of the positive total, 0–1.
    pub fraction: f64,
    /// Shown or toggled off.
    pub visible: bool,
}

/// Pie slices from the latest values. Colours are keyed by series index so pie and legend
/// always agree (the C pie skipped non-positive slices in one place but not the other).
pub fn slices(chart: &ChartData) -> Vec<Slice> {
    let latest: Vec<(usize, &SeriesData, f64)> = chart
        .series
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.last().and_then(|x| x.values.first()).map(|&v| (i, s, v)))
        .collect();
    let total: f64 = latest
        .iter()
        .filter(|(_, s, v)| s.visible && *v > 0.0)
        .map(|(_, _, v)| v)
        .sum();
    latest
        .into_iter()
        .map(|(i, s, v)| Slice {
            index: i,
            name: s.name.clone(),
            color: series_color(i, s.color),
            value: v,
            fraction: if total > 0.0 && v > 0.0 && s.visible {
                v / total
            } else {
                0.0
            },
            visible: s.visible,
        })
        .collect()
}

/// Rectangle in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl Rect {
    /// Point inside?
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Maps data coordinates into a plot rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mapping {
    /// Plot area.
    pub plot: Rect,
    /// X data range.
    pub x: Range,
    /// Y data range.
    pub y: Range,
}

impl Mapping {
    /// Data → pixels.
    pub fn to_px(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.plot.x + self.x.normalize(x) * self.plot.w,
            self.plot.y + self.plot.h - self.y.normalize(y) * self.plot.h,
        )
    }

    /// Pixels → data.
    pub fn to_data(&self, px: f64, py: f64) -> (f64, f64) {
        let fx = if self.plot.w > 0.0 {
            (px - self.plot.x) / self.plot.w
        } else {
            0.0
        };
        let fy = if self.plot.h > 0.0 {
            (self.plot.y + self.plot.h - py) / self.plot.h
        } else {
            0.0
        };
        (self.x.min + fx * self.x.span(), self.y.min + fy * self.y.span())
    }
}

/// The data point closest to a pixel position.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Series index.
    pub series: usize,
    /// Series name.
    pub name: String,
    /// Data X.
    pub x: f64,
    /// Data Y.
    pub y: f64,
    /// Pixel distance.
    pub distance: f64,
}

/// Find the visible point nearest to (`px`, `py`) within `radius` pixels.
pub fn nearest(model: &XyModel, map: &Mapping, px: f64, py: f64, radius: f64) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for s in model.series.iter().filter(|s| s.visible) {
        for &(x, y) in &s.points {
            if x < map.x.min || x > map.x.max {
                continue;
            }
            let (qx, qy) = map.to_px(x, y);
            let d = (qx - px).hypot(qy - py);
            if d <= radius && best.as_ref().is_none_or(|b| d < b.distance) {
                best = Some(Hit {
                    series: s.index,
                    name: s.name.clone(),
                    x,
                    y,
                    distance: d,
                });
            }
        }
    }
    best
}

/// Model for any chart kind.
#[derive(Debug, Clone, PartialEq)]
pub enum Model {
    /// Line chart.
    Line(XyModel),
    /// Scatter chart.
    Scatter(XyModel),
    /// Bar chart.
    Bar(Vec<Bar>),
    /// Pie chart.
    Pie(Vec<Slice>),
}

/// Build the model matching the chart's kind.
pub fn model(chart: &ChartData, view: &ViewState) -> Model {
    match chart.kind {
        ChartKind::Line => Model::Line(xy_model(chart, view)),
        ChartKind::Scatter => Model::Scatter(xy_model(chart, view)),
        ChartKind::Bar => Model::Bar(bars(chart)),
        ChartKind::Pie => Model::Pie(slices(chart)),
    }
}

#[cfg(test)]
mod tests {
    use gtkterm_core::sunshine::SunshineStore;
    use jiff::Timestamp;

    use super::*;

    fn store(lines: &[&str]) -> SunshineStore {
        let mut s = SunshineStore::new();
        for (i, l) in lines.iter().enumerate() {
            s.feed(
                format!("{l}\n").as_bytes(),
                Timestamp::from_millisecond(i as i64 * 500).unwrap(),
            );
        }
        s
    }

    #[test]
    fn line_points_index_and_time_axes() {
        let s = store(&["{a|t|1}", "{a|t|3}", "{a|t|2}"]);
        let c = s.chart("a").unwrap();
        let m = xy_model(c, &ViewState::default());
        assert_eq!(m.series[0].points, vec![(0.0, 1.0), (1.0, 3.0), (2.0, 2.0)]);
        assert_eq!(m.x, Range::new(0.0, 2.0));
        assert!(m.y.min < 1.0 && m.y.max > 3.0);
        let m = xy_model(
            c,
            &ViewState {
                x_axis: XAxis::Time,
                ..ViewState::default()
            },
        );
        assert_eq!(m.series[0].points[2], (1.0, 2.0));
    }

    #[test]
    fn pairs_and_window() {
        let s = store(&["{S:xy|p|1 10 2 20}", "{S:xy|p|3 30}"]);
        let m = xy_model(s.chart("xy").unwrap(), &ViewState::default());
        assert!(m.pairs);
        assert_eq!(m.series[0].points, vec![(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)]);

        let lines: Vec<String> = (0..100).map(|i| format!("{{w|v|{i}}}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let s = store(&refs);
        let view = ViewState {
            x_window: Some(Range::new(10.0, 20.0)),
            ..ViewState::default()
        };
        let m = xy_model(s.chart("w").unwrap(), &view);
        assert_eq!(
            m.series[0].points.first().unwrap().0,
            9.0,
            "one point beyond the left edge"
        );
        assert_eq!(m.series[0].points.last().unwrap().0, 21.0);
        assert!(
            m.y.min > 8.0 && m.y.max < 22.0,
            "y range follows the window: {:?}",
            m.y
        );
    }

    #[test]
    fn downsampling_limits_points() {
        let lines: Vec<String> = (0..1000).map(|i| format!("{{d|v|{}}}", i % 17)).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let s = store(&refs);
        let m = xy_model(
            s.chart("d").unwrap(),
            &ViewState {
                max_points: 100,
                ..ViewState::default()
            },
        );
        assert_eq!(m.series[0].points.len(), 100);
    }

    #[test]
    fn hidden_series_do_not_affect_ranges() {
        let mut s = store(&["{h|a|1|b|1000}", "{h|a|2|b|2000}"]);
        s.chart_mut("h").unwrap().series[1].visible = false;
        let m = xy_model(s.chart("h").unwrap(), &ViewState::default());
        assert!(m.y.max < 10.0, "{:?}", m.y);
    }

    #[test]
    fn bars_from_zero_and_pie_fractions() {
        let s = store(&["{B:b|x|5|y|-2|z|0,0,0|3}"]);
        let b = bars(s.chart("b").unwrap());
        assert_eq!(b.iter().map(|b| b.value).collect::<Vec<_>>(), [5.0, -2.0, 3.0]);
        let r = bar_range(&b);
        assert!(r.min < -2.0 && r.max > 5.0);
        let s = store(&["{P:p|a|1|b|-1|c|3}"]);
        let sl = slices(s.chart("p").unwrap());
        assert_eq!(
            sl.iter().map(|s| s.fraction).collect::<Vec<_>>(),
            [0.25, 0.0, 0.75]
        );
        assert_eq!(
            sl[2].color,
            series_color(2, None),
            "legend and slice share the index colour"
        );
    }

    #[test]
    fn mapping_and_hit_test() {
        let s = store(&["{a|t|0}", "{a|t|10}"]);
        let m = xy_model(s.chart("a").unwrap(), &ViewState::default());
        let map = Mapping {
            plot: Rect {
                x: 10.0,
                y: 10.0,
                w: 100.0,
                h: 100.0,
            },
            x: m.x,
            y: m.y,
        };
        let (px, py) = map.to_px(1.0, 10.0);
        let (dx, dy) = map.to_data(px, py);
        assert!((dx - 1.0).abs() < 1e-9 && (dy - 10.0).abs() < 1e-9);
        let hit = nearest(&m, &map, px + 2.0, py, 8.0).unwrap();
        assert_eq!((hit.x, hit.y), (1.0, 10.0));
        assert!(nearest(&m, &map, 500.0, 500.0, 8.0).is_none());
    }

    #[test]
    fn zoom_and_pan() {
        let full = Range::new(0.0, 100.0);
        let mut v = ViewState::default();
        v.zoom(full, 50.0, 0.5);
        assert_eq!(v.x_window, Some(Range::new(25.0, 75.0)));
        v.pan(full, 10.0);
        assert_eq!(v.x_window, Some(Range::new(35.0, 85.0)));
        v.zoom(full, 50.0, 10.0);
        assert_eq!(
            v.x_window, None,
            "zooming out past the data returns to follow mode"
        );
    }
}
