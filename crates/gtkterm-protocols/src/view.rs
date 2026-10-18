//! Terminal view formatter: renders received bytes as ASCII, hex, binary or decimal dumps.
//!
//! The output layout is byte-compatible with the C version (`gtkterm_terminal_view_*`):
//!
//! * optional index `"%08X: "` at the start of each line,
//! * per byte `"%02X "` (hex), 8 bits + space (binary) or `"%3d "` (decimal),
//! * a line ends when it is full or after a `0x0A` byte; a `0x0A` adds an extra blank line,
//! * mixed modes pad short lines and append `"| "` plus the printable ASCII (`.` otherwise),
//! * lines are terminated with `"\r\n"` because the output is fed to a VTE terminal.
//!
//! Unlike the C version the line state lives in the formatter instance, not in function
//! statics, so several terminals do not share one counter.

use serde::{Deserialize, Serialize};

/// How received data is shown in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// Pass bytes through unchanged.
    #[default]
    Ascii,
    /// Hex dump with a configurable number of bytes per line.
    Hex,
    /// Binary dump sized to the terminal width.
    Binary,
    /// Decimal dump sized to the terminal width.
    Decimal,
    /// Hex dump plus ASCII column, sized to the terminal width.
    Mixed,
    /// Binary dump plus ASCII column.
    BinaryMixed,
    /// Decimal dump plus ASCII column.
    DecimalMixed,
}

impl ViewMode {
    /// All modes in menu order.
    pub const ALL: [Self; 7] = [
        Self::Ascii,
        Self::Hex,
        Self::Binary,
        Self::Decimal,
        Self::Mixed,
        Self::BinaryMixed,
        Self::DecimalMixed,
    ];

    /// Human readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::Hex => "Hexadecimal",
            Self::Binary => "Binary",
            Self::Decimal => "Decimal",
            Self::Mixed => "Mixed (Hex + ASCII)",
            Self::BinaryMixed => "Mixed (Binary + ASCII)",
            Self::DecimalMixed => "Mixed (Decimal + ASCII)",
        }
    }

    fn radix(self) -> Option<Radix> {
        match self {
            Self::Ascii => None,
            Self::Hex | Self::Mixed => Some(Radix::Hex),
            Self::Binary | Self::BinaryMixed => Some(Radix::Binary),
            Self::Decimal | Self::DecimalMixed => Some(Radix::Decimal),
        }
    }

    fn is_mixed(self) -> bool {
        matches!(self, Self::Mixed | Self::BinaryMixed | Self::DecimalMixed)
    }
}

/// Hex widths offered in the menu for plain hex mode.
pub const HEX_WIDTHS: [usize; 5] = [8, 10, 16, 24, 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Radix {
    Hex,
    Binary,
    Decimal,
}

impl Radix {
    /// Characters one byte occupies in the dump column, including the trailing space.
    fn cell_width(self) -> usize {
        match self {
            Self::Hex => 3,
            Self::Binary => 9,
            Self::Decimal => 4,
        }
    }

    fn push(self, out: &mut Vec<u8>, byte: u8) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        match self {
            Self::Hex => {
                out.extend_from_slice(&[HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)], b' ']);
            }
            Self::Binary => {
                let mut cell = [b' '; 9];
                for (i, c) in cell[..8].iter_mut().enumerate() {
                    *c = if byte & (0x80 >> i) == 0 { b'0' } else { b'1' };
                }
                out.extend_from_slice(&cell);
            }
            Self::Decimal => {
                let digit = |d: u8| b'0' + d;
                let cell = match byte {
                    0..=9 => [b' ', b' ', digit(byte), b' '],
                    10..=99 => [b' ', digit(byte / 10), digit(byte % 10), b' '],
                    _ => [digit(byte / 100), digit(byte / 10 % 10), digit(byte % 10), b' '],
                };
                out.extend_from_slice(&cell);
            }
        }
    }
}

/// Settings that determine the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSettings {
    /// Selected mode.
    pub mode: ViewMode,
    /// Print the `%08X: ` byte index at the start of each line.
    pub show_index: bool,
    /// Bytes per line in plain hex mode (the C config key `term_hex_chars`).
    pub hex_chars: usize,
    /// Current terminal width in columns; used by all other dump modes.
    pub columns: usize,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            mode: ViewMode::Ascii,
            show_index: false,
            hex_chars: 16,
            columns: 80,
        }
    }
}

impl ViewSettings {
    /// Bytes per dump line for the current mode, clamped like the C version.
    pub fn bytes_per_line(&self) -> usize {
        let index_width = if self.show_index { 10 } else { 0 };
        let (per_byte, separator, min, max) = match self.mode {
            ViewMode::Ascii => return 0,
            ViewMode::Hex => return self.hex_chars.clamp(1, 128),
            ViewMode::Mixed => (4, 3, 8, 64),
            ViewMode::Binary => (9, 0, 4, 64),
            ViewMode::BinaryMixed => (10, 3, 4, 32),
            ViewMode::Decimal => (4, 0, 8, 80),
            ViewMode::DecimalMixed => (5, 3, 8, 64),
        };
        let available = self.columns.saturating_sub(separator + index_width);
        (available / per_byte).clamp(min, max)
    }
}

/// Stateful formatter; feed it chunks and send the returned bytes to the terminal.
#[derive(Debug, Clone, Default)]
pub struct ViewFormatter {
    settings: ViewSettings,
    byte_index: u64,
    line: Vec<u8>,
}

impl ViewFormatter {
    /// Create a formatter with the given settings.
    pub fn new(settings: ViewSettings) -> Self {
        Self {
            settings,
            byte_index: 0,
            line: Vec::new(),
        }
    }

    /// Current settings.
    pub fn settings(&self) -> ViewSettings {
        self.settings
    }

    /// Change settings. Switching mode starts a fresh line; the byte index keeps counting.
    pub fn set_settings(&mut self, settings: ViewSettings) {
        if settings.mode != self.settings.mode {
            self.line.clear();
        }
        self.settings = settings;
    }

    /// Reset index and line state, e.g. after "clear screen".
    pub fn reset(&mut self) {
        self.reset_to(0);
    }

    /// Start over at byte index `index` (redrawing history that begins there).
    pub fn reset_to(&mut self, index: u64) {
        self.byte_index = index;
        self.line.clear();
    }

    /// Format `data` and return the bytes to feed to the terminal.
    pub fn format(&mut self, data: &[u8]) -> Vec<u8> {
        self.format_stamped(data, None)
    }

    /// Like [`ViewFormatter::format`], starting every new line of this chunk with
    /// `[stamp] ` (the chunk's arrival time, formatted by the caller).
    pub fn format_stamped(&mut self, data: &[u8], stamp: Option<&str>) -> Vec<u8> {
        let Some(radix) = self.settings.mode.radix() else {
            return data.to_vec();
        };
        let mixed = self.settings.mode.is_mixed();
        let per_line = self.settings.bytes_per_line();
        let mut out = Vec::with_capacity(data.len() * (radix.cell_width() + 2));

        for &byte in data {
            if self.line.is_empty() {
                if let Some(stamp) = stamp {
                    out.push(b'[');
                    out.extend_from_slice(stamp.as_bytes());
                    out.extend_from_slice(b"] ");
                }
                if self.settings.show_index {
                    use std::io::Write as _;
                    let _ = write!(out, "{:08X}: ", self.byte_index);
                }
            }
            self.line.push(byte);
            radix.push(&mut out, byte);
            self.byte_index += 1;

            let line_break = byte == 0x0A;
            if line_break || self.line.len() >= per_line {
                if mixed {
                    for _ in self.line.len()..per_line {
                        out.extend(std::iter::repeat_n(b' ', radix.cell_width()));
                    }
                    out.extend_from_slice(b"| ");
                    out.extend(
                        self.line
                            .iter()
                            .map(|&c| if (0x20..=0x7E).contains(&c) { c } else { b'.' }),
                    );
                }
                out.extend_from_slice(b"\r\n");
                if line_break {
                    out.extend_from_slice(b"\r\n");
                }
                self.line.clear();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(mode: ViewMode, show_index: bool, columns: usize, data: &[u8]) -> String {
        let mut f = ViewFormatter::new(ViewSettings {
            mode,
            show_index,
            hex_chars: 8,
            columns,
        });
        String::from_utf8(f.format(data)).expect("formatter emits ASCII")
    }

    #[test]
    fn timestamps_start_each_dump_line() {
        let mut f = ViewFormatter::new(ViewSettings {
            mode: ViewMode::Hex,
            show_index: true,
            hex_chars: 2,
            columns: 80,
        });
        let out = String::from_utf8(f.format_stamped(b"abc", Some("12:00:00.000"))).unwrap();
        assert_eq!(
            out,
            "[12:00:00.000] 00000000: 61 62 \r\n[12:00:00.000] 00000002: 63 "
        );
        // A line continued by the next chunk keeps its first stamp.
        let out = String::from_utf8(f.format_stamped(b"d", Some("12:00:01.000"))).unwrap();
        assert_eq!(out, "64 \r\n");
    }

    #[test]
    fn cells_match_the_c_printf_formats() {
        for byte in 0..=255u8 {
            for (radix, want) in [
                (Radix::Hex, format!("{byte:02X} ")),
                (Radix::Binary, format!("{byte:08b} ")),
                (Radix::Decimal, format!("{byte:3} ")),
            ] {
                let mut out = Vec::new();
                radix.push(&mut out, byte);
                assert_eq!(String::from_utf8(out).unwrap(), want);
                assert_eq!(want.len(), radix.cell_width());
            }
        }
    }

    #[test]
    fn ascii_passes_through() {
        let mut f = ViewFormatter::default();
        assert_eq!(f.format(b"a\x00\xffb\n"), b"a\x00\xffb\n");
    }

    #[test]
    fn hex_wraps_at_hex_chars_and_after_lf() {
        let out = fmt(ViewMode::Hex, false, 80, b"0123456789\nA");
        assert_eq!(out, "30 31 32 33 34 35 36 37 \r\n38 39 0A \r\n\r\n41 ");
    }

    #[test]
    fn hex_index_counts_across_lines_and_chunks() {
        let mut f = ViewFormatter::new(ViewSettings {
            mode: ViewMode::Hex,
            show_index: true,
            hex_chars: 4,
            columns: 80,
        });
        let a = String::from_utf8(f.format(b"abcde")).unwrap();
        let b = String::from_utf8(f.format(b"fgh")).unwrap();
        assert_eq!(a, "00000000: 61 62 63 64 \r\n00000004: 65 ");
        assert_eq!(b, "66 67 68 \r\n");
    }

    #[test]
    fn mixed_pads_and_appends_ascii() {
        // 80 columns, no index: (80-3)/4 = 19 bytes per line.
        let out = fmt(ViewMode::Mixed, false, 80, b"Hi\x01\n");
        let expected = format!("48 69 01 0A {}| Hi..\r\n\r\n", " ".repeat(3 * 15));
        assert_eq!(out, expected);
    }

    #[test]
    fn binary_and_decimal_cells() {
        assert_eq!(
            fmt(ViewMode::Binary, false, 36, b"\x05\xff\x00\x80"),
            "00000101 11111111 00000000 10000000 \r\n"
        );
        assert_eq!(&fmt(ViewMode::Decimal, false, 80, b"\x07\xff")[..8], "  7 255 ");
    }

    #[test]
    fn bytes_per_line_matches_c_clamps() {
        let s = |mode, show_index, columns| {
            ViewSettings {
                mode,
                show_index,
                hex_chars: 16,
                columns,
            }
            .bytes_per_line()
        };
        assert_eq!(s(ViewMode::Hex, false, 10), 16);
        assert_eq!(s(ViewMode::Mixed, false, 80), 19);
        assert_eq!(s(ViewMode::Mixed, true, 80), 16);
        assert_eq!(s(ViewMode::Mixed, false, 10), 8);
        assert_eq!(s(ViewMode::Mixed, false, 1000), 64);
        assert_eq!(s(ViewMode::Binary, false, 80), 8);
        assert_eq!(s(ViewMode::BinaryMixed, false, 80), 7);
        assert_eq!(s(ViewMode::Decimal, false, 80), 20);
        assert_eq!(s(ViewMode::DecimalMixed, true, 80), 13);
    }

    #[test]
    fn instances_do_not_share_state() {
        let settings = ViewSettings {
            mode: ViewMode::Hex,
            show_index: true,
            hex_chars: 2,
            columns: 80,
        };
        let mut a = ViewFormatter::new(settings);
        let mut b = ViewFormatter::new(settings);
        let _ = a.format(b"xyz");
        assert_eq!(String::from_utf8(b.format(b"q")).unwrap(), "00000000: 71 ");
    }
}
