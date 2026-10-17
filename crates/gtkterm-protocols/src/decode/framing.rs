//! Frame assembly helpers.

/// Splits a byte stream into lines at `\r`, `\n` or `\r\n`; empty lines are dropped and lines
/// longer than the cap are discarded up to the next line end.
#[derive(Debug, Clone)]
pub struct LineAssembler {
    line: Vec<u8>,
    max: usize,
    discarding: bool,
}

impl LineAssembler {
    /// Lines longer than `max` bytes are dropped.
    pub fn new(max: usize) -> Self {
        Self {
            line: Vec::new(),
            max: max.max(1),
            discarding: false,
        }
    }

    /// Feed bytes; returns completed lines (without terminators).
    pub fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for &b in data {
            if b == b'\r' || b == b'\n' {
                if !self.discarding && !self.line.is_empty() {
                    out.push(std::mem::take(&mut self.line));
                }
                self.line.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.line.len() == self.max {
                    self.line.clear();
                    self.discarding = true;
                } else {
                    self.line.push(b);
                }
            }
        }
        out
    }

    /// Forget a partial line.
    pub fn reset(&mut self) {
        self.line.clear();
        self.discarding = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_caps() {
        let mut a = LineAssembler::new(5);
        assert_eq!(a.feed(b"ab"), Vec::<Vec<u8>>::new());
        assert_eq!(
            a.feed(b"c\r\n\r\nde\rtoolongline\nok\n"),
            vec![b"abc".to_vec(), b"de".to_vec(), b"ok".to_vec()]
        );
    }
}
