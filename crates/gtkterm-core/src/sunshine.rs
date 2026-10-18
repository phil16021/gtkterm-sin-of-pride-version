//! Sunshine visualization data: one chart per packet ID, ring-buffered series.
//!
//! This is the GTK-free model shared by the chart widgets and the MCP server.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;

use gtkterm_protocols::sunshine::{ChartKind, Packet, Rgb, ScanEvent, Scanner};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Default number of samples kept per series for line/scatter charts.
pub const DEFAULT_CAPACITY: usize = 1000;
/// Default number of samples kept per series for bar/pie charts (only the latest is drawn).
pub const DEFAULT_BAR_CAPACITY: usize = 100;
/// Largest configurable history.
pub const MAX_CAPACITY: usize = 1_000_000;

/// One sample of a series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Per-series running sample number (0, 1, 2, ...), the default X axis.
    pub index: u64,
    /// Time the packet was received.
    pub ts: Timestamp,
    /// Values from the packet (one for Y-only series, two or more for X/Y data).
    pub values: Vec<f64>,
}

/// A named series of a chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesData {
    /// Series name.
    pub name: String,
    /// Colour from the packet, if any.
    pub color: Option<Rgb>,
    /// Ring buffer of samples, oldest first.
    pub samples: VecDeque<Sample>,
    /// Samples ever received (including evicted ones).
    pub total: u64,
    /// Whether the series is shown (toggled from the legend).
    pub visible: bool,
}

impl SeriesData {
    /// Latest sample.
    pub fn last(&self) -> Option<&Sample> {
        self.samples.back()
    }

    /// (min, max) of the first value over retained samples.
    pub fn y_range(&self) -> Option<(f64, f64)> {
        self.samples
            .iter()
            .filter_map(|s| s.values.first().copied())
            .fold(None, |acc, v| match acc {
                None => Some((v, v)),
                Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
            })
    }
}

/// Data of one chart (one packet ID).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartData {
    /// Packet ID.
    pub id: String,
    /// Chart type in use.
    pub kind: ChartKind,
    /// True if the user chose `kind`; packet prefixes no longer change it.
    pub kind_overridden: bool,
    /// Series in order of first appearance.
    pub series: Vec<SeriesData>,
    /// Packets received for this ID.
    pub packets: u64,
    /// While paused, packets are counted but not stored.
    pub paused: bool,
    /// Samples kept per series.
    pub capacity: usize,
    /// Incremented on every change; views redraw when it differs.
    pub revision: u64,
    /// Time of the first packet; origin of the relative time axis.
    pub first_ts: Timestamp,
}

impl ChartData {
    fn new(id: String, kind: ChartKind, ts: Timestamp) -> Self {
        let capacity = default_capacity(kind);
        Self {
            id,
            kind,
            kind_overridden: false,
            series: Vec::new(),
            packets: 0,
            paused: false,
            capacity,
            revision: 0,
            first_ts: ts,
        }
    }

    fn ingest(&mut self, packet: &Packet, ts: Timestamp) {
        self.packets += 1;
        if let Some(kind) = packet.kind.filter(|_| !self.kind_overridden) {
            self.kind = kind;
        }
        if self.paused {
            return;
        }
        for s in &packet.series {
            let idx = self
                .series
                .iter()
                .position(|d| d.name == s.name)
                .unwrap_or_else(|| {
                    self.series.push(SeriesData {
                        name: s.name.clone(),
                        color: s.color,
                        samples: VecDeque::new(),
                        total: 0,
                        visible: true,
                    });
                    self.series.len() - 1
                });
            let data = &mut self.series[idx];
            if s.color.is_some() {
                data.color = s.color;
            }
            data.samples.push_back(Sample {
                index: data.total,
                ts,
                values: s.values.clone(),
            });
            data.total += 1;
            while data.samples.len() > self.capacity {
                data.samples.pop_front();
            }
        }
        self.revision += 1;
    }

    /// Change the history length; trims existing series.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.clamp(1, MAX_CAPACITY);
        for s in &mut self.series {
            while s.samples.len() > self.capacity {
                s.samples.pop_front();
            }
        }
        self.revision += 1;
    }

    /// Remove all samples but keep series names and settings.
    pub fn clear(&mut self) {
        for s in &mut self.series {
            s.samples.clear();
        }
        self.revision += 1;
    }

    /// CSV export: `timestamp,elapsed_s,series,index,value[,value2...]`.
    pub fn to_csv(&self) -> String {
        let mut rows: Vec<(&Sample, &str)> = self
            .series
            .iter()
            .flat_map(|s| s.samples.iter().map(move |x| (x, s.name.as_str())))
            .collect();
        rows.sort_by_key(|(x, _)| (x.ts, x.index));
        let mut out = String::from("timestamp,elapsed_s,series,index,values\n");
        for (x, name) in rows {
            let elapsed = (x.ts.as_millisecond() - self.first_ts.as_millisecond()) as f64 / 1000.0;
            let values = x.values.iter().map(f64::to_string).collect::<Vec<_>>().join(",");
            let _ = writeln!(
                out,
                "{},{elapsed:.3},{},{},{values}",
                x.ts,
                csv_field(name),
                x.index
            );
        }
        out
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn default_capacity(kind: ChartKind) -> usize {
    match kind {
        ChartKind::Line | ChartKind::Scatter => DEFAULT_CAPACITY,
        ChartKind::Bar | ChartKind::Pie => DEFAULT_BAR_CAPACITY,
    }
}

/// Counters for the panel footer and MCP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SunshineStats {
    /// Valid packets.
    pub packets: u64,
    /// Rejected packet lines.
    pub errors: u64,
    /// Lines discarded for exceeding the length limit.
    pub overflows: u64,
    /// Packets dropped because [`MAX_CHARTS`] charts exist already.
    pub too_many_charts: u64,
}

/// Most charts (packet IDs) a session keeps; packets with further IDs are dropped, so line
/// noise or firmware with changing IDs cannot create tabs without end.
pub const MAX_CHARTS: usize = 64;

/// What changed after feeding data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SunshineUpdate {
    /// A new chart was created.
    ChartAdded(String),
    /// New data or settings (type, pause) for an existing chart.
    ChartUpdated(String),
    /// Charts were removed or cleared from outside the window (e.g. by an agent): resync.
    ChartsChanged,
    /// A packet line was rejected.
    Rejected {
        /// The offending line (truncated).
        line: String,
        /// Why.
        error: String,
    },
}

/// All Sunshine charts of a session.
#[derive(Debug, Clone, Default)]
pub struct SunshineStore {
    scanner: Scanner,
    charts: Vec<ChartData>,
    index: HashMap<String, usize>,
    /// User-chosen chart types by ID, applied when the chart is (re)created.
    overrides: HashMap<String, ChartKind>,
    stats: SunshineStats,
    last_error: Option<String>,
}

impl SunshineStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed received bytes; returns what changed (chart updates are coalesced per call).
    pub fn feed(&mut self, data: &[u8], ts: Timestamp) -> Vec<SunshineUpdate> {
        let events = self.scanner.feed(data);
        self.apply(events, ts)
    }

    /// Treat buffered partial data as a full line (port closed).
    pub fn flush(&mut self, ts: Timestamp) -> Vec<SunshineUpdate> {
        let events = self.scanner.flush();
        self.apply(events, ts)
    }

    fn apply(&mut self, events: Vec<ScanEvent>, ts: Timestamp) -> Vec<SunshineUpdate> {
        let mut updates = Vec::new();
        for ev in events {
            match ev {
                ScanEvent::Packet(p) => {
                    if !self.index.contains_key(&p.id) && self.charts.len() >= MAX_CHARTS {
                        self.stats.too_many_charts += 1;
                        if self.stats.too_many_charts == 1 {
                            let error = format!("more than {MAX_CHARTS} packet IDs; further IDs are ignored");
                            self.last_error = Some(error.clone());
                            updates.push(SunshineUpdate::Rejected {
                                line: p.id.clone(),
                                error,
                            });
                        }
                        continue;
                    }
                    self.stats.packets += 1;
                    let (idx, created) = self.chart_index(&p, ts);
                    self.charts[idx].ingest(&p, ts);
                    let update = if created {
                        SunshineUpdate::ChartAdded(p.id.clone())
                    } else {
                        SunshineUpdate::ChartUpdated(p.id.clone())
                    };
                    if created || !updates.contains(&update) {
                        updates.push(update);
                    }
                }
                ScanEvent::Error { line, error } => {
                    self.stats.errors += 1;
                    let error = error.to_string();
                    self.last_error = Some(format!("{error}: {line}"));
                    updates.push(SunshineUpdate::Rejected { line, error });
                }
                ScanEvent::Overflow => self.stats.overflows += 1,
            }
        }
        updates
    }

    fn chart_index(&mut self, p: &Packet, ts: Timestamp) -> (usize, bool) {
        if let Some(&i) = self.index.get(&p.id) {
            return (i, false);
        }
        let mut chart = ChartData::new(p.id.clone(), p.kind.unwrap_or_default(), ts);
        if let Some(&kind) = self.overrides.get(&p.id) {
            chart.kind = kind;
            chart.capacity = default_capacity(kind);
            chart.kind_overridden = true;
        }
        self.charts.push(chart);
        self.index.insert(p.id.clone(), self.charts.len() - 1);
        (self.charts.len() - 1, true)
    }

    /// Charts in creation order.
    pub fn charts(&self) -> &[ChartData] {
        &self.charts
    }

    /// Chart by ID.
    pub fn chart(&self, id: &str) -> Option<&ChartData> {
        self.index.get(id).map(|&i| &self.charts[i])
    }

    /// Mutable chart by ID.
    pub fn chart_mut(&mut self, id: &str) -> Option<&mut ChartData> {
        self.index.get(id).map(|&i| &mut self.charts[i])
    }

    /// Choose the chart type for an ID (also before its first packet arrives).
    pub fn set_kind(&mut self, id: &str, kind: ChartKind) {
        self.overrides.insert(id.to_owned(), kind);
        if let Some(c) = self.chart_mut(id) {
            c.kind = kind;
            c.kind_overridden = true;
            c.revision += 1;
        }
    }

    /// Remove one chart. Returns false if it did not exist.
    pub fn remove(&mut self, id: &str) -> bool {
        let Some(i) = self.index.remove(id) else {
            return false;
        };
        self.charts.remove(i);
        for v in self.index.values_mut() {
            if *v > i {
                *v -= 1;
            }
        }
        true
    }

    /// Remove all charts and reset the scanner; overrides are kept.
    pub fn clear(&mut self) {
        self.charts.clear();
        self.index.clear();
        self.scanner.reset();
    }

    /// Counters.
    pub fn stats(&self) -> SunshineStats {
        self.stats
    }

    /// Last rejected line with reason.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_count_is_capped() {
        let mut s = SunshineStore::new();
        let mut text = String::new();
        for i in 0..(MAX_CHARTS + 10) {
            use std::fmt::Write as _;
            let _ = writeln!(text, "{{id{i}|v|1}}");
        }
        let updates = s.feed(text.as_bytes(), Timestamp::UNIX_EPOCH);
        assert_eq!(s.charts().len(), MAX_CHARTS);
        assert_eq!(s.stats().too_many_charts, 10);
        assert_eq!(
            updates
                .iter()
                .filter(|u| matches!(u, SunshineUpdate::Rejected { .. }))
                .count(),
            1,
            "reported once"
        );
        // Known IDs keep updating.
        s.feed(b"{id0|v|2}\n", Timestamp::UNIX_EPOCH);
        assert_eq!(s.chart("id0").unwrap().packets, 2);
    }

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).unwrap()
    }

    #[test]
    fn routes_by_id_and_prefix() {
        // C defect S5: only line charts were ever created.
        let mut s = SunshineStore::new();
        let u = s.feed(
            b"{P0|a|1}\n{B:P1|x|1|y|2}\n{P:P2|q|3}\n{S:P3|xy|1 2}\n{P0|a|2}\n",
            t(0),
        );
        assert_eq!(
            u.iter()
                .filter(|u| matches!(u, SunshineUpdate::ChartAdded(_)))
                .count(),
            4
        );
        let kinds: Vec<_> = s.charts().iter().map(|c| (c.id.as_str(), c.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("P0", ChartKind::Line),
                ("P1", ChartKind::Bar),
                ("P2", ChartKind::Pie),
                ("P3", ChartKind::Scatter)
            ]
        );
        assert_eq!(s.chart("P0").unwrap().series[0].samples.len(), 2);
        assert_eq!(s.stats().packets, 5);
    }

    #[test]
    fn many_chunks_keep_updating() {
        // C defect S1: 20k small reads must all arrive.
        let mut s = SunshineStore::new();
        for i in 0..20_000 {
            s.feed(format!("{{P0|v|{i}}}\n").as_bytes(), t(i));
        }
        let c = s.chart("P0").unwrap();
        assert_eq!(c.packets, 20_000);
        assert_eq!(c.series[0].total, 20_000);
        assert_eq!(c.series[0].samples.len(), DEFAULT_CAPACITY);
        assert_eq!(c.series[0].last().unwrap().values, [19_999.0]);
    }

    #[test]
    fn split_packets_across_feeds() {
        let mut s = SunshineStore::new();
        assert!(s.feed(b"{P0|T|2", t(0)).is_empty());
        assert_eq!(
            s.feed(b"3.5}\n", t(1)),
            vec![SunshineUpdate::ChartAdded("P0".into())]
        );
    }

    #[test]
    fn overrides_win_over_prefix() {
        let mut s = SunshineStore::new();
        s.set_kind("P0", ChartKind::Scatter);
        s.feed(b"{B:P0|a|1}\n", t(0));
        assert_eq!(s.chart("P0").unwrap().kind, ChartKind::Scatter);
        s.feed(b"{L:P9|a|1}\n{B:P9|a|1}\n", t(0));
        assert_eq!(
            s.chart("P9").unwrap().kind,
            ChartKind::Bar,
            "prefix may change kind until overridden"
        );
    }

    #[test]
    fn pause_capacity_clear_remove() {
        let mut s = SunshineStore::new();
        s.feed(b"{P0|a|1}\n", t(0));
        s.chart_mut("P0").unwrap().paused = true;
        s.feed(b"{P0|a|2}\n", t(1));
        let c = s.chart("P0").unwrap();
        assert_eq!((c.packets, c.series[0].samples.len()), (2, 1));
        s.chart_mut("P0").unwrap().paused = false;
        for i in 0..10 {
            s.feed(format!("{{P0|a|{i}}}\n").as_bytes(), t(i));
        }
        s.chart_mut("P0").unwrap().set_capacity(3);
        assert_eq!(s.chart("P0").unwrap().series[0].samples.len(), 3);
        s.chart_mut("P0").unwrap().clear();
        assert!(s.chart("P0").unwrap().series[0].samples.is_empty());
        s.feed(b"{P1|b|1}\n", t(0));
        assert!(s.remove("P0"));
        assert_eq!(
            s.chart("P1").unwrap().id,
            "P1",
            "index map is fixed up after removal"
        );
        assert!(!s.remove("P0"));
    }

    #[test]
    fn errors_are_counted() {
        let mut s = SunshineStore::new();
        let u = s.feed(b"{P0|a|1,5}\nplain text\n", t(0));
        assert!(matches!(&u[..], [SunshineUpdate::Rejected { .. }]));
        assert_eq!(s.stats().errors, 1);
        assert!(s.last_error().unwrap().contains("decimal separator"));
    }

    #[test]
    fn csv_export() {
        let mut s = SunshineStore::new();
        s.feed(b"{P0|a|1|b,c|0,0,0|2 3}\n", t(1_000));
        s.feed(b"{P0|a|4}\n", t(2_500));
        let csv = s.chart("P0").unwrap().to_csv();
        let lines: Vec<_> = csv.lines().collect();
        assert_eq!(lines[0], "timestamp,elapsed_s,series,index,values");
        assert_eq!(lines[1], "1970-01-01T00:00:01Z,0.000,a,0,1");
        assert_eq!(lines[2], "1970-01-01T00:00:01Z,0.000,\"b,c\",0,2,3");
        assert_eq!(lines[3], "1970-01-01T00:00:02.5Z,1.500,a,1,4");
    }
}
