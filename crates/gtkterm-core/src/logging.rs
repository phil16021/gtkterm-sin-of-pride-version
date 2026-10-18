//! Session log writer.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use gtkterm_protocols::logfmt::{self, LogFormat, LogMeta, LogRecord};
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::chunks::{Chunk, Direction};

/// Logging state, as shown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogState {
    /// Writing.
    Active,
    /// File open, not writing.
    Paused,
}

/// Status of the session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogStatus {
    /// File being written.
    pub path: PathBuf,
    /// Format.
    pub format: LogFormat,
    /// Active or paused.
    pub state: LogState,
    /// Bytes of payload logged.
    pub bytes: u64,
}

/// Writes chunks to a log file.
#[derive(Debug)]
pub struct LogWriter {
    out: BufWriter<File>,
    status: LogStatus,
    records: u64,
    meta: LogMeta,
}

impl LogWriter {
    /// Create (or truncate, if `append` is false) the log file and write the header.
    /// Appending only makes sense for raw and text logs; structured formats always start fresh.
    pub fn create(path: &Path, format: LogFormat, meta: LogMeta, append: bool) -> std::io::Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let append = append && matches!(format, LogFormat::Raw | LogFormat::Text);
        // Logs can hold what was typed into a device console (passwords): readable only by
        // the user, like the MCP audit log and the UI state.
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .mode(0o600)
            .open(path)?;
        let mut out = BufWriter::new(file);
        out.write_all(logfmt::header(format, &meta).as_bytes())?;
        out.flush()?;
        Ok(Self {
            out,
            status: LogStatus {
                path: path.to_owned(),
                format,
                state: LogState::Active,
                bytes: 0,
            },
            records: 0,
            meta,
        })
    }

    /// Current status.
    pub fn status(&self) -> &LogStatus {
        &self.status
    }

    /// Pause or resume.
    pub fn set_paused(&mut self, paused: bool) -> std::io::Result<()> {
        self.status.state = if paused {
            LogState::Paused
        } else {
            LogState::Active
        };
        self.out.flush()
    }

    /// Log one chunk (ignored while paused, and TX chunks in raw logs).
    pub fn write(&mut self, chunk: &Chunk, tz: &TimeZone) -> std::io::Result<()> {
        if self.status.state == LogState::Paused
            || (chunk.dir == Direction::Tx && !self.status.format.logs_tx())
        {
            return Ok(());
        }
        let zoned = chunk.ts.to_zoned(tz.clone());
        let time = match self.status.format {
            LogFormat::Text => zoned.strftime("%H:%M:%S%.3f").to_string(),
            _ => zoned.strftime("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
        };
        let rec = LogRecord {
            time: &time,
            rx: chunk.dir == Direction::Rx,
            data: &chunk.data,
        };
        self.out
            .write_all(&logfmt::record(self.status.format, &rec, self.records == 0))?;
        self.records += 1;
        self.status.bytes += chunk.data.len() as u64;
        Ok(())
    }

    /// Flush buffered output.
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }

    /// Truncate the file and start over with a fresh header (menu "Clear").
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.out.flush()?;
        let file = self.out.get_mut();
        file.set_len(0)?;
        std::io::Seek::rewind(file)?;
        self.out
            .write_all(logfmt::header(self.status.format, &self.meta).as_bytes())?;
        self.records = 0;
        self.status.bytes = 0;
        self.out.flush()
    }

    /// Write the footer and close.
    pub fn finish(mut self) -> std::io::Result<LogStatus> {
        self.out
            .write_all(logfmt::footer(self.status.format).as_bytes())?;
        self.out.flush()?;
        Ok(self.status)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use jiff::Timestamp;

    use super::*;

    fn chunk(dir: Direction, data: &'static [u8]) -> Chunk {
        Chunk {
            peer: None,
            seq: 0,
            offset: 0,
            ts: Timestamp::from_millisecond(1_000).unwrap(),
            dir,
            data: Bytes::from_static(data),
        }
    }

    #[test]
    fn json_log_roundtrip_with_pause_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/s.json");
        let mut w = LogWriter::create(&path, LogFormat::Json, LogMeta::default(), false).unwrap();
        w.write(&chunk(Direction::Rx, b"one"), &TimeZone::UTC).unwrap();
        w.set_paused(true).unwrap();
        w.write(&chunk(Direction::Rx, b"skipped"), &TimeZone::UTC)
            .unwrap();
        w.set_paused(false).unwrap();
        w.write(&chunk(Direction::Tx, b"two"), &TimeZone::UTC).unwrap();
        let status = w.finish().unwrap();
        assert_eq!(status.bytes, 6);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["data"].as_array().unwrap().len(), 2);
        assert_eq!(v["data"][0]["timestamp"], "1970-01-01T00:00:01.000+00:00");

        let mut w = LogWriter::create(&path, LogFormat::Json, LogMeta::default(), true).unwrap();
        w.write(&chunk(Direction::Rx, b"x"), &TimeZone::UTC).unwrap();
        w.clear().unwrap();
        w.write(&chunk(Direction::Rx, b"y"), &TimeZone::UTC).unwrap();
        w.finish().unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["data"][0]["hex"], serde_json::json!([121]));
    }

    #[test]
    fn raw_log_appends_rx_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.log");
        std::fs::write(&path, b"old|").unwrap();
        let mut w = LogWriter::create(&path, LogFormat::Raw, LogMeta::default(), true).unwrap();
        w.write(&chunk(Direction::Rx, b"rx"), &TimeZone::UTC).unwrap();
        w.write(&chunk(Direction::Tx, b"tx"), &TimeZone::UTC).unwrap();
        w.finish().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"old|rx");
    }
}
