//! NMEA 0183 sentences.

use std::fmt::Write as _;

use super::{Check, Frame, Protocol};
use crate::checksum::nmea_xor;

const SENTENCES: &[(&str, &str)] = &[
    ("GGA", "Global Positioning System Fix Data"),
    ("RMC", "Recommended Minimum Navigation Information"),
    ("GSV", "Satellites in View"),
    ("GSA", "GPS DOP and active satellites"),
    ("GLL", "Geographic Position - Latitude/Longitude"),
    ("VTG", "Track made good and Ground speed"),
    ("GBS", "GPS Satellite Fault Detection"),
    ("GST", "GPS Pseudorange Noise Statistics"),
    ("ZDA", "Time & Date"),
    ("HDT", "Heading - True"),
    ("HDM", "Heading - Magnetic"),
    ("DBT", "Depth Below Transducer"),
    ("MTW", "Mean Temperature of Water"),
    ("VHW", "Water speed and heading"),
    ("VLW", "Distance Traveled through Water"),
    ("TXT", "Text message"),
];

const FIX: [&str; 9] = [
    "Invalid",
    "GPS fix",
    "DGPS fix",
    "PPS fix",
    "RTK",
    "Float RTK",
    "Estimated",
    "Manual",
    "Simulation",
];

fn talker(t: &str) -> &'static str {
    match t {
        "GP" => "GPS",
        "GL" => "GLONASS",
        "GA" => "Galileo",
        "GB" | "BD" => "BeiDou",
        "GQ" | "QZ" => "QZSS",
        "GN" => "GNSS",
        "AI" => "AIS",
        "HC" => "Compass",
        "II" => "Instrument",
        _ => "",
    }
}

/// Split `$body*hh` / `!body*hh`; returns (body, checksum check).
fn split(line: &[u8]) -> Option<(&str, Check)> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let body = text.strip_prefix('$').or_else(|| text.strip_prefix('!'))?;
    match body.rsplit_once('*') {
        Some((data, sum)) if sum.len() == 2 => {
            let computed = nmea_xor(data.as_bytes());
            let check = match super::parse_hex(sum.as_bytes()) {
                Some(v) if v == u32::from(computed) => Check::Ok,
                _ => Check::Failed {
                    received: sum.to_ascii_uppercase(),
                    computed: format!("{computed:02X}"),
                },
            };
            Some((data, check))
        }
        _ => Some((body, Check::None)),
    }
}

pub(super) fn looks_valid(line: &[u8]) -> bool {
    matches!(split(line), Some((body, Check::Ok)) if body.len() >= 5)
}

/// `ddmm.mmmm` → decimal degrees (the C version mis-scaled values with != 3 decimals).
fn coord(value: &str, hemi: &str) -> Option<f64> {
    let v: f64 = value.parse().ok()?;
    let deg = (v / 100.0).floor();
    let dec = deg + (v - deg * 100.0) / 60.0;
    Some(if matches!(hemi, "S" | "W") { -dec } else { dec })
}

fn time(s: &str) -> Option<String> {
    (s.len() >= 6 && s.as_bytes()[..6].iter().all(u8::is_ascii_digit))
        .then(|| format!("{}:{}:{}", &s[0..2], &s[2..4], &s[4..6]))
}

pub(super) fn decode(line: &[u8]) -> Option<Frame> {
    let (body, check) = split(line)?;
    let fields: Vec<&str> = body.split(',').collect();
    let head = fields[0];
    if line.trim_ascii_start().first() == Some(&b'!') {
        // AIS: !AIVDM,<count>,<number>,<seq id>,<channel>,<payload>,<fill>
        let part = format!(
            "{}/{}",
            fields.get(2).unwrap_or(&"?"),
            fields.get(1).unwrap_or(&"?")
        );
        let summary = format!(
            "[NMEA] AIS {head} fragment {part} channel {}",
            fields.get(4).unwrap_or(&"")
        );
        return Some(
            Frame::new(Protocol::Nmea, line, summary)
                .field("sentence", head)
                .with_check(check),
        );
    }
    if head.starts_with('P') {
        let summary = format!("[NMEA] Proprietary {head} {}", fields[1..].join(","));
        return Some(
            Frame::new(Protocol::Nmea, line, summary)
                .field("sentence", head)
                .with_check(check),
        );
    }
    if head.len() < 5 || !head.is_ascii() {
        return Some(Frame::new(Protocol::Nmea, line, format!("[NMEA] {body}")).with_check(check));
    }
    let (tk, kind) = head.split_at(2);
    let name = SENTENCES
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("Unknown sentence", |(_, d)| d);
    let who = talker(tk);
    let mut summary = format!(
        "[NMEA] {}{kind} ({name}) ",
        if who.is_empty() {
            String::new()
        } else {
            format!("{who} ")
        }
    );
    let mut frame = Frame::new(Protocol::Nmea, line, String::new())
        .field("talker", tk)
        .field("sentence", kind);
    let get = |i: usize| fields.get(i).copied().unwrap_or("");
    match kind {
        "GGA" => {
            if let Some(t) = time(get(1)) {
                let _ = write!(summary, "Time:{t} ");
                frame = frame.field("time", t);
            }
            if let (Some(lat), Some(lon)) = (coord(get(2), get(3)), coord(get(4), get(5))) {
                let _ = write!(
                    summary,
                    "Pos:{:.6}°{},{:.6}°{} ",
                    lat.abs(),
                    get(3),
                    lon.abs(),
                    get(5)
                );
                frame = frame
                    .field("latitude", format!("{lat:.6}"))
                    .field("longitude", format!("{lon:.6}"));
            }
            if let Some(q) = get(6).parse::<usize>().ok().and_then(|q| FIX.get(q)) {
                let _ = write!(summary, "Fix:{q} ");
                frame = frame.field("fix", *q);
            }
            if !get(7).is_empty() {
                let _ = write!(summary, "Sats:{} ", get(7));
                frame = frame.field("satellites", get(7));
            }
            if !get(9).is_empty() {
                let _ = write!(summary, "Alt:{}m ", get(9));
                frame = frame.field("altitude_m", get(9));
            }
        }
        "RMC" => {
            let status = if get(2) == "A" { "Active" } else { "Void" };
            let _ = write!(summary, "Status:{status} ");
            frame = frame.field("status", status);
            if let Some(t) = time(get(1)) {
                let _ = write!(summary, "Time:{t} ");
                frame = frame.field("time", t);
            }
            if let (Some(lat), Some(lon)) = (coord(get(3), get(4)), coord(get(5), get(6))) {
                let _ = write!(summary, "Pos:{lat:.6}°,{lon:.6}° ");
                frame = frame
                    .field("latitude", format!("{lat:.6}"))
                    .field("longitude", format!("{lon:.6}"));
            }
            if let Ok(sp) = get(7).parse::<f64>() {
                let _ = write!(summary, "Speed:{sp:.1}kt ");
                frame = frame.field("speed_kn", format!("{sp:.1}"));
            }
            if let Ok(c) = get(8).parse::<f64>() {
                let _ = write!(summary, "Course:{c:.1}° ");
                frame = frame.field("course_deg", format!("{c:.1}"));
            }
        }
        "GSV" => {
            let _ = write!(summary, "Msg:{}/{} Sats:{} ", get(2), get(1), get(3));
            frame = frame.field("satellites_in_view", get(3));
        }
        _ => summary.push_str(&fields[1..].join(",")),
    }
    match &check {
        Check::Ok => summary.push_str("[CS:OK]"),
        Check::Failed { received, computed } => {
            let _ = write!(summary, "[CS:FAIL rx:{received} calc:{computed}]");
        }
        Check::None => {}
    }
    summary.truncate(summary.trim_end().len());
    frame.summary = summary;
    Some(frame.with_check(check))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gga_with_correct_coordinates() {
        let f = decode(b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47").unwrap();
        assert_eq!(f.check, Check::Ok);
        assert_eq!(
            f.summary,
            "[NMEA] GPS GGA (Global Positioning System Fix Data) Time:12:35:19 Pos:48.117300°N,11.516667°E Fix:GPS fix Sats:08 Alt:545.4m [CS:OK]"
        );
    }

    #[test]
    fn rmc_south_west_and_bad_checksum() {
        let f = decode(b"$GPRMC,225446,A,4916.45,S,12311.12,W,000.5,054.7,191194,020.3,E*00").unwrap();
        assert!(matches!(f.check, Check::Failed { .. }));
        assert!(
            f.summary.contains("Pos:-49.274167°,-123.185333°"),
            "{}",
            f.summary
        );
        assert!(f.summary.contains("[CS:FAIL rx:00 calc:"));
    }

    #[test]
    fn proprietary_ais_short_and_garbage() {
        assert!(
            decode(b"$PUBX,00,1*00")
                .unwrap()
                .summary
                .starts_with("[NMEA] Proprietary PUBX")
        );
        assert!(
            decode(b"!AIVDM,2,1,3,B,55P5TL01VIaAL@7WKO@mBplU@<PDhh000000001S;AJ::4A80?4i@E53,0*3E")
                .unwrap()
                .summary
                .contains("fragment 1/2")
        );
        assert!(
            decode(b"$GP").is_some(),
            "short sentences decode without panicking"
        );
        assert!(decode(b"no dollar").is_none());
        assert!(looks_valid(
            b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47"
        ));
    }
}
