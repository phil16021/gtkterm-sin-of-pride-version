//! Backslash escapes for text that is sent to devices (macros, scripts).
//!
//! `\r \n \t \0 \e \\` and `\xHH`. Everything else is literal UTF-8.

use std::fmt::Write as _;

use thiserror::Error;

/// Invalid escape sequence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid escape sequence at character {pos}: {reason}")]
pub struct EscapeError {
    /// Character offset of the backslash.
    pub pos: usize,
    /// What is wrong.
    pub reason: &'static str,
}

/// Decode escaped text into bytes.
pub fn decode(text: &str) -> Result<Vec<u8>, EscapeError> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.char_indices().enumerate().peekable();
    while let Some((pos, (_, c))) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        let err = |reason| EscapeError { pos, reason };
        let Some((_, (_, e))) = chars.next() else {
            return Err(err("trailing backslash"));
        };
        match e {
            'r' => out.push(b'\r'),
            'n' => out.push(b'\n'),
            't' => out.push(b'\t'),
            '0' => out.push(0),
            'e' => out.push(0x1B),
            '\\' => out.push(b'\\'),
            'x' => {
                let hi = chars.next().and_then(|(_, (_, h))| h.to_digit(16));
                let lo = chars.next().and_then(|(_, (_, l))| l.to_digit(16));
                match (hi, lo) {
                    (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
                    _ => return Err(err("\\x needs two hex digits")),
                }
            }
            _ => return Err(err("unknown escape (use \\r \\n \\t \\0 \\e \\\\ or \\xHH)")),
        }
    }
    Ok(out)
}

/// Encode bytes as escaped text; `decode(&encode(b)) == b` for all inputs.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    let text = String::from_utf8_lossy(data);
    if matches!(text, std::borrow::Cow::Borrowed(_)) {
        // Valid UTF-8: keep printable characters (including non-ASCII), escape controls.
        for c in text.chars() {
            match c {
                '\r' => out.push_str("\\r"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\\' => out.push_str("\\\\"),
                c if c.is_control() => {
                    let mut buf = [0u8; 4];
                    for b in c.encode_utf8(&mut buf).bytes() {
                        let _ = write!(out, "\\x{b:02X}");
                    }
                }
                c => out.push(c),
            }
        }
    } else {
        for &b in data {
            match b {
                b'\r' => out.push_str("\\r"),
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\\' => out.push_str("\\\\"),
                0x20..=0x7E => out.push(b as char),
                _ => {
                    let _ = write!(out, "\\x{b:02X}");
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn vectors() {
        assert_eq!(decode("ATZ\\r").unwrap(), b"ATZ\r");
        assert_eq!(decode("a\\x41\\x0d\\0\\e\\\\b").unwrap(), b"aA\r\0\x1b\\b");
        assert_eq!(decode("äö").unwrap(), "äö".as_bytes());
        assert_eq!(
            decode("x\\q"),
            Err(EscapeError {
                pos: 1,
                reason: "unknown escape (use \\r \\n \\t \\0 \\e \\\\ or \\xHH)"
            })
        );
        assert!(decode("\\x4").is_err());
        assert!(decode("end\\").is_err());
        assert_eq!(encode(b"AT\r\n\x00\x1b"), "AT\\r\\n\\x00\\x1B");
        assert_eq!(encode("grüß\t".as_bytes()), "grüß\\t");
        assert_eq!(encode(b"\xff\xfe"), "\\xFF\\xFE");
    }

    proptest! {
        #[test]
        fn roundtrip(data in proptest::collection::vec(any::<u8>(), 0..64)) {
            prop_assert_eq!(decode(&encode(&data)).unwrap(), data);
        }

        #[test]
        fn decode_never_panics(s in ".{0,40}") {
            let _ = decode(&s);
        }
    }
}
