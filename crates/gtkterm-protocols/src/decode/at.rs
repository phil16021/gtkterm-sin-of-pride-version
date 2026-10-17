//! Hayes AT commands and responses.

use std::fmt::Write as _;

use super::{Frame, Protocol};

/// (prefix, description); the longest matching prefix wins (the C table matched "AT" first).
const COMMANDS: &[(&str, &str)] = &[
    ("ATA", "Answer incoming call"),
    ("ATD", "Dial number"),
    ("ATE", "Command echo"),
    ("ATH", "Hook control (hang up)"),
    ("ATI", "Request identification"),
    ("ATL", "Speaker loudness"),
    ("ATM", "Speaker mode"),
    ("ATO", "Return to online mode"),
    ("ATQ", "Quiet mode"),
    ("ATS", "S-Register access"),
    ("ATV", "Verbose response mode"),
    ("ATZ", "Reset to default configuration"),
    ("AT&F", "Factory defaults"),
    ("AT&W", "Store configuration"),
    ("AT+CGMI", "Manufacturer identification"),
    ("AT+CGMM", "Model identification"),
    ("AT+CGMR", "Revision identification"),
    ("AT+CGSN", "Serial number"),
    ("AT+GMR", "Firmware version"),
    ("AT+CIMI", "IMSI"),
    ("AT+CCID", "SIM card ID"),
    ("AT+CREG", "Network registration"),
    ("AT+CGREG", "GPRS network registration"),
    ("AT+COPS", "Operator selection"),
    ("AT+CSQ", "Signal quality"),
    ("AT+CPIN", "Enter PIN"),
    ("AT+CMGF", "Message format"),
    ("AT+CMGS", "Send message"),
    ("AT+CMGR", "Read message"),
    ("AT+CMGD", "Delete message"),
    ("AT+CIPSTART", "Start TCP connection"),
    ("AT+CIPCLOSE", "Close TCP connection"),
    ("AT+CIPSEND", "Send TCP data"),
    ("AT+CIPSTATUS", "Connection status"),
    ("AT+CWJAP", "Join Wi-Fi access point"),
    ("AT+CWLAP", "List Wi-Fi access points"),
    ("AT+RST", "Restart module"),
    ("AT", "Attention"),
];

const INDICATIONS: &[&str] = &[
    "RING",
    "CONNECT",
    "NO CARRIER",
    "BUSY",
    "NO DIALTONE",
    "NO ANSWER",
    "READY",
    "SMS Ready",
    "Call Ready",
];

pub(super) fn looks_like(line: &[u8]) -> bool {
    let t = String::from_utf8_lossy(line);
    let t = t.trim();
    let u = t.to_ascii_uppercase();
    u.starts_with("AT")
        || matches!(u.as_str(), "OK" | "ERROR")
        || u.starts_with("+CME ERROR")
        || u.starts_with("+CMS ERROR")
        || (t.starts_with('+') && t.contains(':'))
        || INDICATIONS.iter().any(|i| u.starts_with(&i.to_ascii_uppercase()))
}

pub(super) fn decode(line: &[u8]) -> Option<Frame> {
    let text = String::from_utf8_lossy(line);
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let upper = t.to_ascii_uppercase();
    let frame = |summary: String| Frame::new(Protocol::At, line, summary);
    if upper.starts_with("AT") {
        let (name, desc) = COMMANDS
            .iter()
            .filter(|(p, _)| upper.starts_with(p))
            .max_by_key(|(p, _)| p.len())
            .map_or(("AT", "Attention"), |&(p, d)| (p, d));
        let rest = &t[name.len()..];
        let (kind, arg) = if rest == "?" {
            ("read", String::new())
        } else if rest == "=?" {
            ("test", String::new())
        } else if let Some(v) = rest.strip_prefix('=') {
            ("set", v.to_owned())
        } else if rest.is_empty() {
            ("execute", String::new())
        } else {
            ("execute", rest.to_owned())
        };
        let mut summary = format!("[CMD] {name} ({desc})");
        match kind {
            "read" => summary.push_str(" ?"),
            "test" => summary.push_str(" =? (supported values)"),
            _ if !arg.is_empty() => _ = write!(summary, " (Set: {arg})"),
            _ => {}
        }
        let f = frame(summary).field("command", name).field("kind", kind);
        return Some(if arg.is_empty() {
            f
        } else {
            f.field("argument", arg)
        });
    }
    if upper == "OK" {
        return Some(frame("[RSP] OK".into()).field("result", "OK"));
    }
    if upper == "ERROR" {
        return Some(frame("[ERR] ERROR".into()).field("result", "ERROR"));
    }
    for (prefix, what) in [
        ("+CME ERROR:", "equipment error"),
        ("+CMS ERROR:", "message service error"),
    ] {
        if upper.starts_with(prefix) {
            let code = t[prefix.len()..].trim();
            return Some(
                frame(format!("[ERR] {} ({what} {code})", &t[..prefix.len() - 1])).field("error_code", code),
            );
        }
    }
    if let Some(ind) = INDICATIONS
        .iter()
        .find(|i| upper.starts_with(&i.to_ascii_uppercase()))
    {
        return Some(frame(format!("[IND] {t}")).field("indication", *ind));
    }
    if let Some((name, value)) = t.strip_prefix('+').and_then(|r| r.split_once(':')) {
        return Some(
            frame(format!("[RSP] +{name}: {}", value.trim()))
                .field("response", format!("+{name}"))
                .field("value", value.trim()),
        );
    }
    Some(frame(format!("[TXT] {t}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(line: &str) -> String {
        decode(line.as_bytes()).unwrap().summary
    }

    #[test]
    fn commands_use_the_longest_prefix() {
        assert_eq!(s("AT+CSQ"), "[CMD] AT+CSQ (Signal quality)");
        assert_eq!(s("AT+CREG?"), "[CMD] AT+CREG (Network registration) ?");
        assert_eq!(s("AT+CMGF=1"), "[CMD] AT+CMGF (Message format) (Set: 1)");
        assert_eq!(
            s("AT+COPS=?"),
            "[CMD] AT+COPS (Operator selection) =? (supported values)"
        );
        assert_eq!(s("ATZ"), "[CMD] ATZ (Reset to default configuration)");
        assert_eq!(s("AT"), "[CMD] AT (Attention)");
        assert_eq!(s("atd+43123"), "[CMD] ATD (Dial number) (Set: +43123)");
    }

    #[test]
    fn responses() {
        assert_eq!(s("OK"), "[RSP] OK");
        assert_eq!(s("ERROR"), "[ERR] ERROR");
        assert_eq!(s("+CME ERROR: 10"), "[ERR] +CME ERROR (equipment error 10)");
        assert_eq!(s("+CSQ: 23,99"), "[RSP] +CSQ: 23,99");
        assert_eq!(s("NO CARRIER"), "[IND] NO CARRIER");
        assert_eq!(s("Quectel EC25"), "[TXT] Quectel EC25");
        assert!(decode(b"  ").is_none());
    }
}
