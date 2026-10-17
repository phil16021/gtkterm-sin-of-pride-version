//! Signal monitor: a time-bounded history of modem line changes.
//!
//! Lines are polled; the kernel's transition counters (`TIOCGICOUNT`) reveal input pulses that
//! started and ended between two polls, which are recorded as a toggle and its return.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::port::{LineCounters, ModemLines};

/// How long changes are kept.
pub const HISTORY: Duration = Duration::from_secs(300);

/// Line names in display order.
pub const NAMES: [&str; 6] = ["DTR", "RTS", "CTS", "DSR", "DCD", "RI"];

/// Levels of [`NAMES`] in order.
pub fn levels(m: &ModemLines) -> [bool; 6] {
    [m.dtr, m.rts, m.cts, m.dsr, m.dcd, m.ri]
}

/// One state of the lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalSample {
    /// When the state was observed.
    pub time: Timestamp,
    /// Line levels.
    pub lines: ModemLines,
    /// Lines that changed since the previous sample (empty for the first one).
    pub changed: Vec<&'static str>,
    /// Part of a pulse shorter than the poll interval (seen only through the counters).
    pub pulse: bool,
}

/// Bounded history of line states.
#[derive(Debug, Default)]
pub struct SignalHistory {
    samples: VecDeque<SignalSample>,
    counters: Option<LineCounters>,
    paused: bool,
}

impl SignalHistory {
    /// Record the current lines (and counters, when the driver has them). Returns the new samples.
    pub fn observe(
        &mut self,
        time: Timestamp,
        lines: ModemLines,
        counters: Option<LineCounters>,
    ) -> Vec<SignalSample> {
        if self.paused {
            return Vec::new();
        }
        let mut new = Vec::new();
        let prev = self.samples.back().map(|s| s.lines);
        match prev {
            None => new.push(SignalSample {
                time,
                lines,
                changed: Vec::new(),
                pulse: false,
            }),
            Some(prev) => {
                // Inputs that toggled an even number of times since the last poll: short pulses.
                if let (Some(old), Some(cur)) = (self.counters, counters) {
                    let pulses = [
                        ("CTS", prev.cts == lines.cts, cur.cts.saturating_sub(old.cts)),
                        ("DSR", prev.dsr == lines.dsr, cur.dsr.saturating_sub(old.dsr)),
                        ("DCD", prev.dcd == lines.dcd, cur.dcd.saturating_sub(old.dcd)),
                        ("RI", prev.ri == lines.ri, cur.ri.saturating_sub(old.ri)),
                    ];
                    for (name, same, delta) in pulses {
                        if same && delta >= 2 {
                            let mut flipped = prev;
                            match name {
                                "CTS" => flipped.cts = !flipped.cts,
                                "DSR" => flipped.dsr = !flipped.dsr,
                                "DCD" => flipped.dcd = !flipped.dcd,
                                _ => flipped.ri = !flipped.ri,
                            }
                            for l in [flipped, prev] {
                                new.push(SignalSample {
                                    time,
                                    lines: l,
                                    changed: vec![name],
                                    pulse: true,
                                });
                            }
                        }
                    }
                }
                if prev != lines {
                    let (a, b) = (levels(&prev), levels(&lines));
                    new.push(SignalSample {
                        time,
                        lines,
                        changed: NAMES
                            .iter()
                            .zip(a.iter().zip(b))
                            .filter(|(_, (x, y))| *x != y)
                            .map(|(n, _)| *n)
                            .collect(),
                        pulse: false,
                    });
                }
            }
        }
        if counters.is_some() {
            self.counters = counters;
        }
        self.samples.extend(new.iter().cloned());
        self.prune(time);
        new
    }

    fn prune(&mut self, now: Timestamp) {
        let cutoff = now - jiff::SignedDuration::try_from(HISTORY).unwrap_or_default();
        // Keep one sample older than the window: it is the state at the window's start.
        while self.samples.len() > 1 && self.samples[1].time < cutoff {
            self.samples.pop_front();
        }
    }

    /// Samples newer than `since` plus the state just before it.
    pub fn since(&self, since: Option<Timestamp>) -> Vec<SignalSample> {
        let Some(since) = since else {
            return self.samples.iter().cloned().collect();
        };
        let first_new = self
            .samples
            .iter()
            .position(|s| s.time >= since)
            .unwrap_or(self.samples.len());
        self.samples
            .iter()
            .skip(first_new.saturating_sub(1))
            .cloned()
            .collect()
    }

    /// Latest state.
    pub fn current(&self) -> Option<&SignalSample> {
        self.samples.back()
    }

    /// Stop or resume recording.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Recording is paused.
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Forget everything (the next observation becomes the first sample).
    pub fn clear(&mut self) {
        self.samples.clear();
        self.counters = None;
    }

    /// `Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals` (the C signal monitor's CSV).
    pub fn to_csv(&self, tz: &TimeZone) -> String {
        let mut out = String::from("Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals\n");
        for s in &self.samples {
            let t = s.time.to_zoned(tz.clone()).strftime("%Y-%m-%d %H:%M:%S%.3f");
            let _ = write!(out, "{t}");
            for v in levels(&s.lines) {
                let _ = write!(out, ",{}", u8::from(v));
            }
            let mut changed = s.changed.join(" ");
            if s.pulse {
                changed.push_str(" (pulse)");
            }
            let _ = writeln!(out, ",{changed}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(1_700_000_000_000 + ms).unwrap()
    }

    #[test]
    fn changes_pulses_and_window() {
        let mut h = SignalHistory::default();
        let mut l = ModemLines::default();
        let c = LineCounters::default();
        assert_eq!(h.observe(t(0), l, Some(c)).len(), 1, "initial state");
        assert!(h.observe(t(50), l, Some(c)).is_empty(), "no change");
        l.dtr = true;
        l.cts = true;
        let c1 = LineCounters { cts: 1, ..c };
        let new = h.observe(t(100), l, Some(c1));
        assert_eq!(new[0].changed, ["DTR", "CTS"]);
        // RI rang between two polls.
        let new = h.observe(t(150), l, Some(LineCounters { ri: 2, ..c1 }));
        assert_eq!(new.len(), 2);
        assert!(new[0].lines.ri && !new[1].lines.ri && new[0].pulse);
        assert_eq!(h.since(Some(t(120))).len(), 3, "the state before plus the pulse");
        // 300 s later only the last state before the window remains.
        l.rts = true;
        h.observe(t(400_000), l, None);
        let all = h.since(None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].changed, ["RTS"]);
        let csv = h.to_csv(&TimeZone::UTC);
        assert!(csv.starts_with("Timestamp,DTR,RTS,CTS,DSR,DCD,RI,Changed Signals\n"));
        assert!(csv.lines().last().unwrap().ends_with(",1,1,1,0,0,0,RTS"), "{csv}");
        h.set_paused(true);
        l.dsr = true;
        assert!(h.observe(t(400_100), l, None).is_empty());
    }
}
