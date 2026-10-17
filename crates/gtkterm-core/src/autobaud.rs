//! Experimental baud rate detection: listen at each candidate rate and score what arrives by the
//! share of printable text, penalised by framing, parity, overrun and break errors.

use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::config::Connection;
use crate::session::{Session, SessionError};

/// Rates tried by default, most common first.
pub const CANDIDATES: [u32; 11] = [
    115_200, 9600, 19_200, 38_400, 57_600, 230_400, 460_800, 921_600, 4800, 2400, 1_000_000,
];

/// Result for one rate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BaudScore {
    /// Rate.
    pub baudrate: u32,
    /// Bytes received while listening.
    pub bytes: usize,
    /// Share of printable ASCII and whitespace.
    pub printable: f32,
    /// Framing + parity + overrun + break errors reported by the driver.
    pub errors: u64,
    /// 0.0–1.0.
    pub score: f32,
}

/// Outcome of a detection run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AutoBaud {
    /// Chosen rate (the port is left open at it); `None` if nothing looked like text.
    pub best: Option<u32>,
    /// All rates tried.
    pub scores: Vec<BaudScore>,
}

/// Score received bytes: printable share, reduced by the error rate.
pub fn score(data: &[u8], errors: u64) -> (f32, f32) {
    if data.len() < 8 {
        return (0.0, 0.0);
    }
    let printable = data
        .iter()
        .filter(|&&b| (0x20..0x7F).contains(&b) || matches!(b, b'\r' | b'\n' | b'\t'))
        .count() as f32
        / data.len() as f32;
    let error_rate = errors as f32 / data.len() as f32;
    (printable, printable * (1.0 - (error_rate * 4.0).min(1.0)))
}

/// Try `candidates` (listening `listen` at each) on the session's serial connection and reopen
/// it at the best rate, or at the original rate if none scored at least 0.6. Needs a device
/// that is sending text.
pub async fn auto_baud(
    session: &Session,
    candidates: &[u32],
    listen: Duration,
    cancel: &CancellationToken,
) -> Result<AutoBaud, SessionError> {
    let Some(Connection::Serial(original)) = session.connection() else {
        return Err(SessionError::Transfer(
            "auto-baud needs a serial connection".into(),
        ));
    };
    let mut scores = Vec::new();
    let errors = |s: &Session| {
        s.status()
            .counters
            .map_or(0, |c| c.frame + c.parity + c.overrun + c.brk)
    };
    for &rate in candidates {
        if cancel.is_cancelled() {
            break;
        }
        let mut cfg = original.clone();
        cfg.baudrate = rate;
        session.open(Connection::Serial(cfg)).await?;
        let start = session.rx_cursor();
        // Counters arrive with the first modem poll after opening.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let e0 = errors(session);
        let deadline = tokio::time::Instant::now() + listen;
        while tokio::time::Instant::now() < deadline && session.read_rx(start, 4096).data.len() < 512 {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
        let data = session.read_rx(start, 4096).data;
        let errs = errors(session).saturating_sub(e0);
        let (printable, s) = score(&data, errs);
        scores.push(BaudScore {
            baudrate: rate,
            bytes: data.len(),
            printable,
            errors: errs,
            score: s,
        });
        if s >= 0.98 && data.len() >= 64 {
            break;
        }
    }
    let best = scores
        .iter()
        .filter(|s| s.score >= 0.6)
        .max_by(|a, b| a.score.total_cmp(&b.score).then(a.bytes.cmp(&b.bytes)))
        .map(|s| s.baudrate);
    let mut cfg = original;
    if let Some(b) = best {
        cfg.baudrate = b;
    }
    session.open(Connection::Serial(cfg)).await?;
    Ok(AutoBaud { best, scores })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring() {
        assert_eq!(score(b"short", 0).1, 0.0);
        assert!((score(b"hello world\r\n", 0).1 - 1.0).abs() < 1e-6);
        let garbage: Vec<u8> = (0..64u8).map(|i| 0x80 | i).collect();
        assert!(score(&garbage, 0).1 < 0.1);
        assert!(
            score(b"hello world, how are you\r\n", 10).1 < 0.1,
            "errors dominate"
        );
    }
}
