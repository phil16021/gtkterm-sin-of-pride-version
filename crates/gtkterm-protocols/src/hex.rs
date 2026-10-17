//! Parsing of user-entered hex byte strings for "send hex data".
//!
//! Accepted forms, freely mixed: `DE AD be ef`, `0xDE 0xAD`, `\xDE\xAD`, `de:ad-be,ef`,
//! and unbroken runs like `DEADBEEF`. Separators are whitespace, `:`, `-`, `,` and `;`.

use thiserror::Error;

/// A hex input error with the character offset where it was found.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HexError {
    /// A character that is neither a hex digit nor a separator.
    #[error("invalid character {ch:?} at position {pos}")]
    InvalidChar {
        /// Offending character.
        ch: char,
        /// Character offset (0-based).
        pos: usize,
    },
    /// A run of digits with odd length, e.g. `ABC`.
    #[error("odd number of hex digits in group starting at position {pos}")]
    OddDigits {
        /// Character offset of the group start.
        pos: usize,
    },
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, ':' | '-' | ',' | ';')
}

/// Parse a hex string into bytes.
pub fn parse_hex(input: &str) -> Result<Vec<u8>, HexError> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::with_capacity(chars.len() / 2);
    let mut i = 0;
    while i < chars.len() {
        if is_separator(chars[i]) {
            i += 1;
            continue;
        }
        // Optional `0x` / `\x` prefix.
        if matches!(chars[i], '0' | '\\') && matches!(chars.get(i + 1), Some('x' | 'X')) {
            i += 2;
        }
        let start = i;
        while chars.get(i).is_some_and(char::is_ascii_hexdigit) {
            i += 1;
        }
        match chars.get(i) {
            Some(&ch) if !is_separator(ch) && ch != '\\' => return Err(HexError::InvalidChar { ch, pos: i }),
            Some('\\') if i == start => return Err(HexError::InvalidChar { ch: '\\', pos: i }),
            _ => {}
        }
        let digits = &chars[start..i];
        if digits.is_empty() || !digits.len().is_multiple_of(2) {
            return Err(HexError::OddDigits { pos: start });
        }
        for pair in digits.chunks(2) {
            let hi = pair[0].to_digit(16).expect("validated hex digit");
            let lo = pair[1].to_digit(16).expect("validated hex digit");
            out.push((hi * 16 + lo) as u8);
        }
    }
    Ok(out)
}

/// Format bytes as space separated upper-case hex, the inverse of [`parse_hex`].
pub fn to_hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn accepted_forms() {
        let want = vec![0xDE, 0xAD, 0xBE, 0xEF];
        for input in [
            "DE AD BE EF",
            "deadbeef",
            "0xDE 0xAD 0xbe 0xEF",
            "\\xDE\\xAD\\xBE\\xEF",
            "de:ad-be,ef",
            " DE\tAD\nBE;EF ",
        ] {
            assert_eq!(parse_hex(input), Ok(want.clone()), "{input}");
        }
        assert_eq!(parse_hex(""), Ok(vec![]));
    }

    #[test]
    fn errors_have_positions() {
        assert_eq!(parse_hex("DE AG"), Err(HexError::InvalidChar { ch: 'G', pos: 4 }));
        assert_eq!(parse_hex("ABC"), Err(HexError::OddDigits { pos: 0 }));
        assert_eq!(parse_hex("01 0x"), Err(HexError::OddDigits { pos: 5 }));
        assert_eq!(parse_hex("zz"), Err(HexError::InvalidChar { ch: 'z', pos: 0 }));
    }

    proptest! {
        #[test]
        fn roundtrip(data in proptest::collection::vec(any::<u8>(), 0..64)) {
            prop_assert_eq!(parse_hex(&to_hex(&data)), Ok(data));
        }
    }
}
