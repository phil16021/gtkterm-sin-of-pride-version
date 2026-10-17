//! CAN frames in candump text and Lawicel SLCAN form.

use std::fmt::Write as _;

use super::{Frame, Protocol, hex, parse_hex};

fn hex_byte(s: &[u8]) -> Option<u8> {
    if s.len() > 2 {
        return None;
    }
    parse_hex(s).and_then(|v| u8::try_from(v).ok())
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let s: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'.')
        .collect();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_chunks::<2>().0.iter().map(|p| hex_byte(p)).collect()
}

fn frame(
    protocol: Protocol,
    raw: &[u8],
    tag: &str,
    (id, extended): (u32, bool),
    rtr: bool,
    data: &[u8],
    iface: Option<&str>,
) -> Frame {
    let width = if extended { 8 } else { 3 };
    let mut summary = format!("[{tag}] ");
    if let Some(i) = iface {
        let _ = write!(summary, "{i} ");
    }
    let _ = write!(
        summary,
        "ID:0x{id:0width$X}{} DLC:{}",
        if extended { " EXT" } else { "" },
        data.len()
    );
    if rtr {
        summary.push_str(" RTR");
    } else if !data.is_empty() {
        let _ = write!(summary, " Data: {}", hex(data));
    }
    Frame::new(protocol, raw, summary)
        .field("id", format!("0x{id:0width$X}"))
        .field(
            "format",
            if extended {
                "extended (29 bit)"
            } else {
                "standard (11 bit)"
            },
        )
        .field("dlc", data.len().to_string())
        .field("rtr", rtr.to_string())
        .field("data", hex(data))
}

fn parse_id(s: &str) -> Option<(u32, bool)> {
    // The ID width decides standard vs extended (the C decoder guessed from the value).
    let id = parse_hex(s.as_bytes())?;
    match s.len() {
        1..=3 if id <= 0x7FF => Some((id, false)),
        8 if id <= 0x1FFF_FFFF => Some((id, true)),
        _ => None,
    }
}

/// `can0  123   [2]  11 22`, `(1234.5) can0 123#1122`, `123#R`, `1F334455#DEADBEEF`.
pub(super) fn decode_candump(line: &[u8]) -> Option<Frame> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if words
        .first()
        .is_some_and(|w| w.starts_with('(') && w.ends_with(')'))
    {
        words.remove(0);
    }
    let iface = words
        .first()
        .filter(|w| !w.contains('#') && !w.chars().all(|c| c.is_ascii_hexdigit()))
        .copied();
    if iface.is_some() {
        words.remove(0);
    }
    let first = *words.first()?;
    if let Some((id, payload)) = first.split_once('#') {
        let (id, extended) = parse_id(id)?;
        if payload.eq_ignore_ascii_case("R") || payload.starts_with(['R', 'r']) {
            return Some(frame(
                Protocol::Can,
                line,
                "CAN",
                (id, extended),
                true,
                &[],
                iface,
            ));
        }
        let data = hex_bytes(payload)?;
        if data.len() > 64 {
            return None;
        }
        return Some(frame(
            Protocol::Can,
            line,
            "CAN",
            (id, extended),
            false,
            &data,
            iface,
        ));
    }
    // candump default: ID [DLC] bytes...
    let (id, extended) = parse_id(first)?;
    let dlc = words
        .get(1)?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<usize>()
        .ok()?;
    if words.get(2).is_some_and(|w| w.eq_ignore_ascii_case("remote")) {
        return Some(frame(
            Protocol::Can,
            line,
            "CAN",
            (id, extended),
            true,
            &[],
            iface,
        ));
    }
    let data: Option<Vec<u8>> = words
        .iter()
        .skip(2)
        .take(dlc)
        .map(|w| hex_byte(w.as_bytes()))
        .collect();
    let data = data?;
    (data.len() == dlc).then(|| frame(Protocol::Can, line, "CAN", (id, extended), false, &data, iface))
}

pub(super) fn candump_looks_like(line: &[u8]) -> bool {
    decode_candump(line).is_some()
}

/// SLCAN: `tIIILDD..`, `TIIIIIIIILDD..`, `rIIIL`, `RIIIIIIIIL`, optional 4-digit timestamp.
pub(super) fn decode_slcan(line: &[u8]) -> Option<Frame> {
    let text = std::str::from_utf8(line)
        .ok()?
        .trim_matches(|c: char| c == '\x07' || c.is_whitespace());
    let kind = text.chars().next()?;
    let (extended, rtr) = match kind {
        't' => (false, false),
        'T' => (true, false),
        'r' => (false, true),
        'R' => (true, true),
        _ => return None,
    };
    let id_len = if extended { 8 } else { 3 };
    let id_str = text.get(1..1 + id_len)?;
    let (id, _) = parse_id(id_str)?;
    let dlc = text
        .get(1 + id_len..2 + id_len)?
        .parse::<usize>()
        .ok()
        .filter(|d| *d <= 8)?;
    let rest = &text[2 + id_len..];
    let data = if rtr {
        Vec::new()
    } else {
        hex_bytes(rest.get(..dlc * 2)?)?
    };
    let tail = if rtr { rest } else { &rest[dlc * 2..] };
    if !(tail.is_empty() || (tail.len() == 4 && tail.chars().all(|c| c.is_ascii_hexdigit()))) {
        return None;
    }
    let mut f = frame(Protocol::Slcan, line, "SLCAN", (id, extended), rtr, &data, None);
    if rtr {
        f.summary = f.summary.replace("DLC:0", &format!("DLC:{dlc}"));
    }
    if tail.len() == 4 {
        let ms = u16::from_str_radix(tail, 16).unwrap_or(0);
        let _ = write!(f.summary, " t={ms}ms");
        f = f.field("timestamp_ms", ms.to_string());
    }
    Some(f)
}

pub(super) fn slcan_looks_like(line: &[u8]) -> bool {
    decode_slcan(line).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(line: &str) -> Option<String> {
        decode_candump(line.as_bytes()).map(|f| f.summary)
    }

    #[test]
    fn candump_forms() {
        assert_eq!(
            s("  can0  123   [4]  DE AD BE EF").unwrap(),
            "[CAN] can0 ID:0x123 DLC:4 Data: DE AD BE EF"
        );
        assert_eq!(
            s("(1617.123456) vcan0 1F334455#0102").unwrap(),
            "[CAN] vcan0 ID:0x1F334455 EXT DLC:2 Data: 01 02"
        );
        assert_eq!(
            s("123#DEADBEEF").unwrap(),
            "[CAN] ID:0x123 DLC:4 Data: DE AD BE EF",
            "compact form (C swallowed it into the ID)"
        );
        assert_eq!(s("7FF#R").unwrap(), "[CAN] ID:0x7FF DLC:0 RTR");
        assert_eq!(s("  can0  001   [0]").unwrap(), "[CAN] can0 ID:0x001 DLC:0");
        assert!(s("800#00").is_none(), "11-bit IDs above 0x7FF are invalid");
        assert!(s("can0 123 [2] 11").is_none(), "DLC mismatch");
        assert!(s("hello world").is_none());
        // Fuzz finding: a multi-byte character where a hex pair was expected panicked.
        assert!(s("A#^\u{FFFD}").is_none());
        assert!(s("+12#00").is_none(), "signs are not hex digits");
        assert!(s("can0 123 [1] +1").is_none());
    }

    #[test]
    fn slcan_forms() {
        let f = |l: &str| decode_slcan(l.as_bytes()).map(|f| f.summary);
        assert_eq!(f("t1232AABB").unwrap(), "[SLCAN] ID:0x123 DLC:2 Data: AA BB");
        assert_eq!(
            f("T1F3344558DEADBEEF00112233").unwrap(),
            "[SLCAN] ID:0x1F334455 EXT DLC:8 Data: DE AD BE EF 00 11 22 33"
        );
        assert_eq!(f("r1234").unwrap(), "[SLCAN] ID:0x123 DLC:4 RTR");
        assert_eq!(
            f("t1231011223").unwrap(),
            "[SLCAN] ID:0x123 DLC:1 Data: 01 t=4643ms"
        );
        assert!(f("t1239").is_none(), "DLC > 8");
        assert!(f("t12").is_none());
        assert!(f("V1013").is_none());
    }
}
