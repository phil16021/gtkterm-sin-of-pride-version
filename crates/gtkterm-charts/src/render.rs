//! Cairo + Pango rendering of Sunshine charts.

use gtkterm_core::sunshine::ChartData;
use gtkterm_protocols::sunshine::ChartKind;
use pango::FontDescription;

use crate::model::{Bar, Hit, Mapping, Model, Rect, Slice, ViewState, XAxis, XyModel, bar_range, model};
use crate::palette::Color;
use crate::scale::{Range, format_tick, ticks};

/// Colours and font of a chart.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    /// Text and axes.
    pub fg: Color,
    /// Background.
    pub bg: Color,
    /// Font family/size, Pango syntax ("Sans 9").
    pub font: String,
}

impl Style {
    /// Light theme.
    pub fn light() -> Self {
        Self {
            fg: (0.13, 0.13, 0.15),
            bg: (1.0, 1.0, 1.0),
            font: "Sans 9".into(),
        }
    }

    /// Dark theme.
    pub fn dark() -> Self {
        Self {
            fg: (0.92, 0.92, 0.94),
            bg: (0.14, 0.14, 0.16),
            font: "Sans 9".into(),
        }
    }

    fn grid(&self) -> (f64, f64, f64, f64) {
        (self.fg.0, self.fg.1, self.fg.2, 0.12)
    }

    fn muted(&self) -> (f64, f64, f64, f64) {
        (self.fg.0, self.fg.1, self.fg.2, 0.6)
    }
}

/// What was drawn, for hit testing in the widget.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Plot area.
    pub plot: Rect,
    /// Data-to-pixel mapping (line and scatter charts).
    pub mapping: Option<Mapping>,
    /// Legend entries: clickable area and series index.
    pub legend: Vec<(Rect, usize)>,
    /// The model that was drawn.
    pub model: Model,
}

impl Layout {
    /// Series index of the legend entry at a pixel position.
    pub fn legend_at(&self, x: f64, y: f64) -> Option<usize> {
        self.legend
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, i)| *i)
    }

    /// Nearest data point (line/scatter) within `radius` pixels.
    pub fn hit(&self, x: f64, y: f64, radius: f64) -> Option<Hit> {
        let map = self.mapping.as_ref()?;
        match &self.model {
            Model::Line(m) | Model::Scatter(m) => crate::model::nearest(m, map, x, y, radius),
            _ => None,
        }
    }
}

/// Rendering errors.
#[derive(Debug)]
pub enum RenderError {
    /// Cairo failure.
    Cairo(cairo::Error),
    /// PNG encoding failure.
    Png(String),
}

impl std::error::Error for RenderError {}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cairo(e) => write!(f, "cairo: {e}"),
            Self::Png(e) => write!(f, "png: {e}"),
        }
    }
}

impl From<cairo::Error> for RenderError {
    fn from(e: cairo::Error) -> Self {
        Self::Cairo(e)
    }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

struct Painter<'a> {
    cr: &'a cairo::Context,
    font: FontDescription,
    bold: FontDescription,
}

impl Painter<'_> {
    fn rgba(&self, c: (f64, f64, f64, f64)) {
        self.cr.set_source_rgba(c.0, c.1, c.2, c.3);
    }

    fn rgb(&self, c: Color) {
        self.cr.set_source_rgb(c.0, c.1, c.2);
    }

    fn layout(&self, text: &str, bold: bool) -> pango::Layout {
        let l = pangocairo::functions::create_layout(self.cr);
        l.set_font_description(Some(if bold { &self.bold } else { &self.font }));
        l.set_text(text);
        l
    }

    fn measure(&self, text: &str) -> (f64, f64) {
        let (w, h) = self.layout(text, false).pixel_size();
        (f64::from(w), f64::from(h))
    }

    /// Draw text with its top edge at `y`; returns its size.
    fn text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        align: Align,
        bold: bool,
        max_width: Option<f64>,
    ) -> (f64, f64) {
        let l = self.layout(text, bold);
        if let Some(mw) = max_width {
            l.set_width((mw.max(1.0) * f64::from(pango::SCALE)) as i32);
            l.set_ellipsize(pango::EllipsizeMode::End);
        }
        let (w, h) = l.pixel_size();
        let (w, h) = (f64::from(w), f64::from(h));
        let x = match align {
            Align::Left => x,
            Align::Center => x - w / 2.0,
            Align::Right => x - w,
        };
        self.cr.move_to(x, y);
        pangocairo::functions::show_layout(self.cr, &l);
        (w, h)
    }
}

fn kind_label(k: ChartKind) -> &'static str {
    k.label()
}

/// Draw `chart` into a `width` × `height` area of `cr`.
pub fn draw(
    cr: &cairo::Context,
    chart: &ChartData,
    view: &ViewState,
    style: &Style,
    width: f64,
    height: f64,
    hover: Option<&Hit>,
) -> Result<Layout, cairo::Error> {
    let font = FontDescription::from_string(&style.font);
    let mut bold = font.clone();
    bold.set_weight(pango::Weight::Bold);
    let p = Painter { cr, font, bold };
    cr.save()?;
    p.rgb(style.bg);
    cr.paint()?;

    let (_, line_h) = p.measure("0");
    let pad = (line_h * 0.6).round();
    p.rgb(style.fg);
    let title = format!(
        "{}  ·  {}{}",
        chart.id,
        kind_label(chart.kind),
        if chart.paused { "  ·  paused" } else { "" }
    );
    p.text(&title, pad, pad, Align::Left, true, Some(width - 2.0 * pad));
    p.rgba(style.muted());
    p.text(
        &format!("{} packets", chart.packets),
        width - pad,
        pad,
        Align::Right,
        false,
        None,
    );

    let content = Rect {
        x: pad,
        y: pad * 2.0 + line_h,
        w: (width - 2.0 * pad).max(1.0),
        h: (height - pad * 3.0 - line_h).max(1.0),
    };
    let m = model(chart, view);
    let empty = match &m {
        Model::Line(x) | Model::Scatter(x) => x.series.iter().all(|s| s.points.is_empty()),
        Model::Bar(b) => b.is_empty(),
        Model::Pie(s) => s.is_empty(),
    };
    if empty {
        p.rgba(style.muted());
        p.text(
            "Waiting for data…",
            width / 2.0,
            height / 2.0 - line_h / 2.0,
            Align::Center,
            false,
            None,
        );
        cr.restore()?;
        return Ok(Layout {
            plot: content,
            mapping: None,
            legend: Vec::new(),
            model: m,
        });
    }

    let layout = match &m {
        Model::Line(x) => draw_xy(&p, style, content, x, view, false, hover)?,
        Model::Scatter(x) => draw_xy(&p, style, content, x, view, true, hover)?,
        Model::Bar(b) => draw_bars(&p, style, content, b)?,
        Model::Pie(s) => draw_pie(&p, style, content, s)?,
    };
    cr.restore()?;
    Ok(Layout { model: m, ..layout })
}

fn grid_y(p: &Painter<'_>, style: &Style, plot: Rect, y: Range) -> Result<(), cairo::Error> {
    let t = ticks(y, ((plot.h / 45.0) as usize).clamp(2, 10));
    for v in &t.values {
        let py = plot.y + plot.h - y.normalize(*v) * plot.h;
        p.rgba(style.grid());
        p.cr.set_line_width(1.0);
        p.cr.move_to(plot.x, py.round() + 0.5);
        p.cr.line_to(plot.x + plot.w, py.round() + 0.5);
        p.cr.stroke()?;
        p.rgba(style.muted());
        let label = format_tick(*v, t.step);
        let (_, h) = p.measure(&label);
        p.text(&label, plot.x - 6.0, py - h / 2.0, Align::Right, false, None);
    }
    Ok(())
}

fn y_label_width(p: &Painter<'_>, y: Range, plot_h: f64) -> f64 {
    let t = ticks(y, ((plot_h / 45.0) as usize).clamp(2, 10));
    t.values
        .iter()
        .map(|v| p.measure(&format_tick(*v, t.step)).0)
        .fold(0.0, f64::max)
}

#[allow(clippy::too_many_arguments)]
fn legend(
    p: &Painter<'_>,
    style: &Style,
    entries: &[(usize, String, Color, bool)],
    right: f64,
    top: f64,
    max_w: f64,
) -> Result<Vec<(Rect, usize)>, cairo::Error> {
    let (_, lh) = p.measure("Ag");
    let sw = lh * 0.8;
    let width = entries
        .iter()
        .map(|e| p.measure(&e.1).0)
        .fold(0.0, f64::max)
        .min(max_w)
        + sw
        + 18.0;
    let height = entries.len() as f64 * (lh + 2.0) + 8.0;
    let x = right - width;
    p.cr.set_source_rgba(style.bg.0, style.bg.1, style.bg.2, 0.85);
    p.cr.rectangle(x, top, width, height);
    p.cr.fill_preserve()?;
    p.rgba(style.grid());
    p.cr.set_line_width(1.0);
    p.cr.stroke()?;
    let mut hits = Vec::new();
    for (row, (index, name, color, visible)) in entries.iter().enumerate() {
        let y = top + 4.0 + row as f64 * (lh + 2.0);
        p.cr.set_source_rgba(color.0, color.1, color.2, if *visible { 1.0 } else { 0.25 });
        p.cr.rectangle(x + 6.0, y + (lh - sw) / 2.0, sw, sw);
        p.cr.fill()?;
        let fg = if *visible {
            (style.fg.0, style.fg.1, style.fg.2, 1.0)
        } else {
            style.muted()
        };
        p.rgba(fg);
        p.text(
            name,
            x + 12.0 + sw,
            y,
            Align::Left,
            false,
            Some(width - sw - 18.0),
        );
        hits.push((
            Rect {
                x,
                y,
                w: width,
                h: lh + 2.0,
            },
            *index,
        ));
    }
    Ok(hits)
}

fn draw_xy(
    p: &Painter<'_>,
    style: &Style,
    area: Rect,
    m: &XyModel,
    view: &ViewState,
    scatter: bool,
    hover: Option<&Hit>,
) -> Result<Layout, cairo::Error> {
    let (_, lh) = p.measure("0");
    let left = y_label_width(p, m.y, area.h) + 10.0;
    let plot = Rect {
        x: area.x + left,
        y: area.y,
        w: (area.w - left).max(1.0),
        h: (area.h - lh * 2.0 - 6.0).max(1.0),
    };
    grid_y(p, style, plot, m.y)?;

    let xt = ticks(m.x, ((plot.w / 90.0) as usize).clamp(2, 12));
    for v in &xt.values {
        let px = plot.x + m.x.normalize(*v) * plot.w;
        p.rgba(style.grid());
        p.cr.move_to(px.round() + 0.5, plot.y);
        p.cr.line_to(px.round() + 0.5, plot.y + plot.h);
        p.cr.stroke()?;
        p.rgba(style.muted());
        p.text(
            &format_tick(*v, xt.step),
            px,
            plot.y + plot.h + 4.0,
            Align::Center,
            false,
            None,
        );
    }
    let axis = if m.pairs {
        "x"
    } else {
        match view.x_axis {
            XAxis::Index => "sample",
            XAxis::Time => "time [s]",
        }
    };
    let zoom = if view.x_window.is_some() {
        "   (zoomed — double-click to follow)"
    } else {
        ""
    };
    p.text(
        &format!("{axis}{zoom}"),
        plot.x + plot.w,
        plot.y + plot.h + 4.0 + lh,
        Align::Right,
        false,
        None,
    );

    p.rgb(style.fg);
    p.cr.set_line_width(1.0);
    p.cr.rectangle(plot.x + 0.5, plot.y + 0.5, plot.w - 1.0, plot.h - 1.0);
    p.cr.stroke()?;

    let map = Mapping { plot, x: m.x, y: m.y };
    p.cr.save()?;
    p.cr.rectangle(plot.x, plot.y, plot.w, plot.h);
    p.cr.clip();
    for s in m.series.iter().filter(|s| s.visible && !s.points.is_empty()) {
        p.rgb(s.color);
        if scatter {
            for &(x, y) in &s.points {
                let (px, py) = map.to_px(x, y);
                p.cr.arc(px, py, 3.0, 0.0, std::f64::consts::TAU);
                p.cr.fill()?;
            }
        } else {
            p.cr.set_line_width(1.6);
            p.cr.set_line_join(cairo::LineJoin::Round);
            let mut first = true;
            for &(x, y) in &s.points {
                let (px, py) = map.to_px(x, y);
                if first {
                    p.cr.move_to(px, py);
                    first = false;
                } else {
                    p.cr.line_to(px, py);
                }
            }
            if s.points.len() == 1 {
                let (px, py) = map.to_px(s.points[0].0, s.points[0].1);
                p.cr.arc(px, py, 2.5, 0.0, std::f64::consts::TAU);
                p.cr.fill()?;
            } else {
                p.cr.stroke()?;
            }
        }
    }
    if let Some(h) = hover {
        let (px, py) = map.to_px(h.x, h.y);
        p.rgb(style.fg);
        p.cr.set_line_width(1.5);
        p.cr.arc(px, py, 5.0, 0.0, std::f64::consts::TAU);
        p.cr.stroke()?;
    }
    p.cr.restore()?;

    // Legend with the newest value and the min…max of what is visible.
    let entries: Vec<_> = m
        .series
        .iter()
        .map(|s| {
            let label = match s.stats.filter(|_| !m.pairs) {
                Some(st) => format!(
                    "{}  {}  ({} … {}, ⌀ {})",
                    s.name,
                    short(st.last),
                    short(st.min),
                    short(st.max),
                    short(st.mean)
                ),
                None => s.name.clone(),
            };
            (s.index, label, s.color, s.visible)
        })
        .collect();
    let legend = legend(
        p,
        style,
        &entries,
        plot.x + plot.w - 6.0,
        plot.y + 6.0,
        plot.w * 0.5,
    )?;

    if let Some(h) = hover {
        let (px, py) = map.to_px(h.x, h.y);
        let label = format!("{}  x={}  y={}", h.name, short(h.x), short(h.y));
        let (tw, th) = p.measure(&label);
        let bx = (px + 10.0).min(plot.x + plot.w - tw - 8.0).max(plot.x);
        let by = (py - th - 12.0).max(plot.y);
        p.cr.set_source_rgba(style.fg.0, style.fg.1, style.fg.2, 0.9);
        p.cr.rectangle(bx, by, tw + 8.0, th + 4.0);
        p.cr.fill()?;
        p.rgb(style.bg);
        p.text(&label, bx + 4.0, by + 2.0, Align::Left, false, None);
    }
    Ok(Layout {
        plot,
        mapping: Some(map),
        legend,
        model: Model::Line(m.clone()),
    })
}

fn short(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn draw_bars(p: &Painter<'_>, style: &Style, area: Rect, bars: &[Bar]) -> Result<Layout, cairo::Error> {
    let (_, lh) = p.measure("0");
    let range = bar_range(bars);
    let left = y_label_width(p, range, area.h) + 10.0;
    let plot = Rect {
        x: area.x + left,
        y: area.y + lh,
        w: (area.w - left).max(1.0),
        h: (area.h - lh * 2.0 - 8.0).max(1.0),
    };
    grid_y(p, style, plot, range)?;
    let visible: Vec<&Bar> = bars.iter().filter(|b| b.visible).collect();
    let n = visible.len().max(1) as f64;
    let slot = plot.w / n;
    let zero_y = plot.y + plot.h - range.normalize(0.0) * plot.h;
    for (i, b) in visible.iter().enumerate() {
        let cx = plot.x + slot * (i as f64 + 0.5);
        let bw = (slot * 0.7).max(1.0);
        let vy = plot.y + plot.h - range.normalize(b.value) * plot.h;
        p.rgb(b.color);
        p.cr.rectangle(cx - bw / 2.0, vy.min(zero_y), bw, (zero_y - vy).abs().max(1.0));
        p.cr.fill()?;
        p.rgb(style.fg);
        let label_y = if b.value >= 0.0 { vy - lh - 2.0 } else { vy + 2.0 };
        p.text(&short(b.value), cx, label_y, Align::Center, false, Some(slot));
        p.rgba(style.muted());
        p.text(
            &b.name,
            cx,
            plot.y + plot.h + 4.0,
            Align::Center,
            false,
            Some(slot - 4.0),
        );
    }
    p.rgb(style.fg);
    p.cr.set_line_width(1.0);
    p.cr.move_to(plot.x, zero_y.round() + 0.5);
    p.cr.line_to(plot.x + plot.w, zero_y.round() + 0.5);
    p.cr.stroke()?;
    let entries: Vec<_> = bars
        .iter()
        .map(|b| (b.index, b.name.clone(), b.color, b.visible))
        .collect();
    let legend = legend(p, style, &entries, area.x + area.w, area.y, area.w * 0.35)?;
    Ok(Layout {
        plot,
        mapping: None,
        legend,
        model: Model::Bar(bars.to_vec()),
    })
}

fn draw_pie(p: &Painter<'_>, style: &Style, area: Rect, slices: &[Slice]) -> Result<Layout, cairo::Error> {
    let entries: Vec<_> = slices
        .iter()
        .map(|s| {
            let label = if s.fraction > 0.0 {
                format!("{}  {} ({:.1}%)", s.name, short(s.value), s.fraction * 100.0)
            } else {
                format!("{}  {} (–)", s.name, short(s.value))
            };
            (s.index, label, s.color, s.visible)
        })
        .collect();
    let legend_w = entries.iter().map(|e| p.measure(&e.1).0).fold(0.0, f64::max) + 40.0;
    let side = area.w > area.h * 1.3;
    let pie_area = if side {
        Rect {
            w: (area.w - legend_w.min(area.w * 0.5)).max(1.0),
            ..area
        }
    } else {
        Rect {
            h: (area.h * 0.7).max(1.0),
            ..area
        }
    };
    let r = (pie_area.w.min(pie_area.h) * 0.42).max(4.0);
    let (cx, cy) = (pie_area.x + pie_area.w / 2.0, pie_area.y + pie_area.h / 2.0);
    let mut angle = -std::f64::consts::FRAC_PI_2;
    if slices.iter().all(|s| s.fraction == 0.0) {
        p.rgba(style.grid());
        p.cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
        p.cr.fill()?;
    }
    for s in slices.iter().filter(|s| s.fraction > 0.0) {
        let end = angle + s.fraction * std::f64::consts::TAU;
        p.rgb(s.color);
        p.cr.move_to(cx, cy);
        p.cr.arc(cx, cy, r, angle, end);
        p.cr.close_path();
        p.cr.fill_preserve()?;
        p.rgb(style.bg);
        p.cr.set_line_width(1.5);
        p.cr.stroke()?;
        if s.fraction >= 0.06 {
            let mid = f64::midpoint(angle, end);
            let label = format!("{:.0}%", s.fraction * 100.0);
            let (_, h) = p.measure(&label);
            p.text(
                &label,
                cx + mid.cos() * r * 0.65,
                cy + mid.sin() * r * 0.65 - h / 2.0,
                Align::Center,
                true,
                None,
            );
        }
        angle = end;
    }
    let (right, top) = if side {
        (area.x + area.w, area.y + 4.0)
    } else {
        (
            area.x + area.w / 2.0 + legend_w / 2.0,
            pie_area.y + pie_area.h + 4.0,
        )
    };
    let legend = legend(p, style, &entries, right, top, legend_w)?;
    Ok(Layout {
        plot: pie_area,
        mapping: None,
        legend,
        model: Model::Pie(slices.to_vec()),
    })
}

/// Render a chart to PNG bytes (headless export, MCP).
pub fn render_png(
    chart: &ChartData,
    view: &ViewState,
    style: &Style,
    width: i32,
    height: i32,
) -> Result<Vec<u8>, RenderError> {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width.max(16), height.max(16))?;
    {
        let cr = cairo::Context::new(&surface)?;
        draw(
            &cr,
            chart,
            view,
            style,
            f64::from(width.max(16)),
            f64::from(height.max(16)),
            None,
        )?;
    }
    let mut out = Vec::new();
    surface
        .write_to_png(&mut out)
        .map_err(|e| RenderError::Png(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use gtkterm_core::sunshine::SunshineStore;
    use jiff::Timestamp;

    use super::*;

    fn chart(lines: &str, id: &str) -> ChartData {
        let mut s = SunshineStore::new();
        for (i, l) in lines.lines().enumerate() {
            s.feed(
                format!("{l}\n").as_bytes(),
                Timestamp::from_millisecond(i as i64 * 100).unwrap(),
            );
        }
        s.chart(id).unwrap().clone()
    }

    /// Count pixels that differ clearly from the background.
    fn ink(chart: &ChartData, style: &Style) -> (usize, Layout) {
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 400, 300).unwrap();
        let layout = {
            let cr = cairo::Context::new(&surface).unwrap();
            draw(&cr, chart, &ViewState::default(), style, 400.0, 300.0, None).unwrap()
        };
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let bg = [
            (style.bg.2 * 255.0) as u8,
            (style.bg.1 * 255.0) as u8,
            (style.bg.0 * 255.0) as u8,
        ];
        let mut n = 0;
        for y in 0..300 {
            for x in 0..400 {
                let px = &data[y * stride + x * 4..y * stride + x * 4 + 3];
                if px.iter().zip(bg).any(|(a, b)| a.abs_diff(b) > 40) {
                    n += 1;
                }
            }
        }
        (n, layout)
    }

    #[test]
    fn every_kind_draws_something_in_both_themes() {
        let line: String = (0..200)
            .map(|i| format!("{{L:c|a|{}|b|{}}}\n", f64::from(i).sin(), f64::from(i).cos()))
            .collect();
        let cases = [
            (chart(&line, "c"), 3000),
            (chart("{B:c|x|5|y|-2|z|3}", "c"), 3000),
            (chart("{P:c|x|5|y|2|z|3}", "c"), 10000),
            (chart("{S:c|p|1 1 2 4 3 9 4 16}", "c"), 400),
        ];
        for (c, min_ink) in &cases {
            for style in [Style::light(), Style::dark()] {
                let (n, layout) = ink(c, &style);
                assert!(n > *min_ink, "{:?} drew only {n} pixels", c.kind);
                assert!(!layout.legend.is_empty(), "{:?} has a legend", c.kind);
            }
        }
    }

    #[test]
    fn legend_and_hover_hit_testing() {
        let c = chart("{c|a|1}\n{c|a|5}\n{c|a|2}", "c");
        let (_, layout) = ink(&c, &Style::light());
        let (rect, idx) = layout.legend[0];
        assert_eq!(layout.legend_at(rect.x + 2.0, rect.y + 2.0), Some(idx));
        let map = layout.mapping.unwrap();
        let (px, py) = map.to_px(1.0, 5.0);
        let hit = layout.hit(px + 1.0, py + 1.0, 6.0).unwrap();
        assert_eq!((hit.x, hit.y), (1.0, 5.0));
    }

    #[test]
    fn empty_and_tiny_surfaces_do_not_fail() {
        let mut s = SunshineStore::new();
        s.feed(b"{e|a|1}\n", Timestamp::UNIX_EPOCH);
        s.chart_mut("e").unwrap().clear();
        let c = s.chart("e").unwrap();
        let png = render_png(c, &ViewState::default(), &Style::light(), 3, 3).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let c = chart("{c|a|1}", "c");
        render_png(&c, &ViewState::default(), &Style::dark(), 20, 20).unwrap();
    }

    #[test]
    fn png_export_writes_a_file() {
        let c = chart("{P:share|a|1|b|2}", "share");
        let png = render_png(&c, &ViewState::default(), &Style::light(), 640, 400).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.png"), &png).unwrap();
        assert!(png.len() > 2000);
    }
}
