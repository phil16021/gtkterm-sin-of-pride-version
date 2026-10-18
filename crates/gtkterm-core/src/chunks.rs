//! History of received and transmitted data.
//!
//! Chunks are kept in arrival order, capped by total bytes and by count. Every chunk carries a
//! monotonic sequence number, and received bytes additionally a monotonic byte offset, so
//! readers (terminal views, MCP `read`) can resume from a cursor and notice when data was
//! evicted. Eviction never shifts indices that readers rely on (ROADMAP defect S1).

use std::collections::VecDeque;
use std::net::SocketAddr;

use bytes::Bytes;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Direction of a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Received from the device.
    Rx,
    /// Sent to the device.
    Tx,
}

/// One received or transmitted block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Sequence number, unique and increasing within a session.
    pub seq: u64,
    /// Offset of the first byte within the direction's byte stream.
    pub offset: u64,
    /// Arrival / transmission time.
    pub ts: Timestamp,
    /// Direction.
    pub dir: Direction,
    /// Payload.
    pub data: Bytes,
    /// The TCP client that sent it (TCP server connections), if known.
    pub peer: Option<SocketAddr>,
}

/// Received bytes of one TCP client within an [`RxRead`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerSpan {
    /// Stream offset of the first byte.
    pub offset: u64,
    /// Number of bytes.
    pub len: usize,
    /// The client.
    pub peer: SocketAddr,
}

/// Result of [`ChunkStore::read_rx`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RxRead {
    /// Bytes read.
    pub data: Vec<u8>,
    /// Cursor to pass next time.
    pub next_cursor: u64,
    /// Bytes between the requested cursor and the oldest retained data that were evicted.
    pub lost: u64,
    /// Which TCP client sent which bytes (empty unless the data came from a TCP server's clients).
    pub peers: Vec<PeerSpan>,
}

/// Bounded chunk history.
#[derive(Debug, Clone)]
pub struct ChunkStore {
    chunks: VecDeque<Chunk>,
    /// RX stream position after each chunk (monotonic, so `read_rx` can binary-search).
    rx_end: VecDeque<u64>,
    bytes: usize,
    max_bytes: usize,
    max_chunks: usize,
    next_seq: u64,
    rx_total: u64,
    tx_total: u64,
}

impl Default for ChunkStore {
    fn default() -> Self {
        Self::new(16 * 1024 * 1024, 200_000)
    }
}

impl ChunkStore {
    /// Store keeping at most `max_bytes` payload bytes and `max_chunks` chunks.
    pub fn new(max_bytes: usize, max_chunks: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            rx_end: VecDeque::new(),
            bytes: 0,
            max_bytes: max_bytes.max(1),
            max_chunks: max_chunks.max(1),
            next_seq: 0,
            rx_total: 0,
            tx_total: 0,
        }
    }

    /// Append a chunk and evict old ones as needed. Returns the stored chunk.
    pub fn push(&mut self, dir: Direction, data: Bytes, ts: Timestamp) -> Chunk {
        self.push_from(dir, data, ts, None)
    }

    /// [`ChunkStore::push`] for data from a known TCP client.
    pub fn push_from(
        &mut self,
        dir: Direction,
        data: Bytes,
        ts: Timestamp,
        peer: Option<SocketAddr>,
    ) -> Chunk {
        let offset = match dir {
            Direction::Rx => &mut self.rx_total,
            Direction::Tx => &mut self.tx_total,
        };
        let chunk = Chunk {
            seq: self.next_seq,
            offset: *offset,
            ts,
            dir,
            data,
            peer,
        };
        *offset += chunk.data.len() as u64;
        self.next_seq += 1;
        self.bytes += chunk.data.len();
        self.chunks.push_back(chunk.clone());
        self.rx_end.push_back(self.rx_total);
        while self.chunks.len() > 1 && (self.bytes > self.max_bytes || self.chunks.len() > self.max_chunks) {
            if let Some(old) = self.chunks.pop_front() {
                self.rx_end.pop_front();
                self.bytes -= old.data.len();
            }
        }
        chunk
    }

    /// Number of retained chunks.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// True if nothing is retained.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Retained payload bytes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Total bytes ever received.
    pub fn rx_total(&self) -> u64 {
        self.rx_total
    }

    /// Total bytes ever transmitted.
    pub fn tx_total(&self) -> u64 {
        self.tx_total
    }

    /// Sequence number the next chunk will get.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Retained chunks in order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Chunk> + ExactSizeIterator {
        self.chunks.iter()
    }

    /// Chunks with `seq >= from`. If some were evicted, iteration starts at the oldest retained.
    pub fn since(&self, from: u64) -> impl Iterator<Item = &Chunk> {
        let start = self
            .chunks
            .front()
            .map_or(0, |c| from.saturating_sub(c.seq) as usize);
        self.chunks.iter().skip(start)
    }

    /// Read up to `max` received bytes starting at byte `cursor` of the RX stream.
    pub fn read_rx(&self, cursor: u64, max: usize) -> RxRead {
        let mut data = Vec::new();
        let mut peers: Vec<PeerSpan> = Vec::new();
        let mut pos = cursor.min(self.rx_total);
        // The oldest retained RX byte: the first chunk's offset, or the RX position after a
        // leading TX chunk.
        let oldest = match self.chunks.front() {
            Some(c) if c.dir == Direction::Rx => c.offset,
            Some(_) => self.rx_end[0],
            None => self.rx_total,
        };
        let lost = oldest.saturating_sub(pos);
        pos = pos.max(oldest);
        // Skip chunks that end before `pos` without walking them.
        let first = self.rx_end.partition_point(|&end| end <= pos);
        for c in self.chunks.range(first..).filter(|c| c.dir == Direction::Rx) {
            let end = c.offset + c.data.len() as u64;
            if end <= pos {
                continue;
            }
            if data.len() >= max {
                break;
            }
            let skip = (pos - c.offset) as usize;
            let take = (c.data.len() - skip).min(max - data.len());
            data.extend_from_slice(&c.data[skip..skip + take]);
            if let Some(peer) = c.peer {
                match peers.last_mut() {
                    Some(span) if span.peer == peer && span.offset + span.len as u64 == pos => {
                        span.len += take;
                    }
                    _ => peers.push(PeerSpan {
                        offset: pos,
                        len: take,
                        peer,
                    }),
                }
            }
            pos += take as u64;
        }
        RxRead {
            data,
            next_cursor: pos,
            lost,
            peers,
        }
    }

    /// Drop all retained data; counters and sequence numbers keep increasing.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.rx_end.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn reads_report_which_tcp_client_sent_what() {
        let a: SocketAddr = "10.0.0.2:5000".parse().unwrap();
        let b: SocketAddr = "10.0.0.3:5001".parse().unwrap();
        let mut s = ChunkStore::new(1 << 20, 100);
        s.push_from(Direction::Rx, Bytes::from_static(b"aa"), ts(), Some(a));
        s.push_from(Direction::Rx, Bytes::from_static(b"aaa"), ts(), Some(a));
        s.push(Direction::Tx, Bytes::from_static(b"tx"), ts());
        s.push_from(Direction::Rx, Bytes::from_static(b"bb"), ts(), Some(b));
        let r = s.read_rx(1, 100);
        assert_eq!(r.data, b"aaaabb");
        assert_eq!(
            r.peers,
            [
                PeerSpan {
                    offset: 1,
                    len: 4,
                    peer: a
                },
                PeerSpan {
                    offset: 5,
                    len: 2,
                    peer: b
                }
            ]
        );
        let mut plain = ChunkStore::new(1 << 20, 100);
        plain.push(Direction::Rx, Bytes::from_static(b"x"), ts());
        assert!(plain.read_rx(0, 10).peers.is_empty());
    }

    fn ts() -> Timestamp {
        Timestamp::UNIX_EPOCH
    }

    #[test]
    fn eviction_by_count_keeps_sequence_monotonic() {
        // C defect S1: after 10k chunks the C store stopped advancing.
        let mut s = ChunkStore::new(usize::MAX, 10_000);
        for i in 0..20_000u32 {
            s.push(Direction::Rx, Bytes::from(i.to_le_bytes().to_vec()), ts());
        }
        assert_eq!(s.len(), 10_000);
        assert_eq!(s.next_seq(), 20_000);
        assert_eq!(s.iter().next().unwrap().seq, 10_000);
        assert_eq!(s.since(19_998).count(), 2);
        assert_eq!(
            s.since(0).count(),
            10_000,
            "evicted range starts at oldest retained"
        );
    }

    #[test]
    fn eviction_by_bytes() {
        let mut s = ChunkStore::new(10, 100);
        for _ in 0..5 {
            s.push(Direction::Rx, Bytes::from_static(b"abcd"), ts());
        }
        assert!(s.bytes() <= 10);
        assert_eq!(s.len(), 2);
        // A single oversized chunk is still kept.
        s.push(Direction::Rx, Bytes::from(vec![0; 100]), ts());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn read_rx_cursor_skips_tx_and_reports_loss() {
        let mut s = ChunkStore::new(8, 100);
        s.push(Direction::Rx, Bytes::from_static(b"hello"), ts());
        s.push(Direction::Tx, Bytes::from_static(b"AT"), ts());
        let r = s.read_rx(0, 100);
        assert_eq!((r.data.as_slice(), r.next_cursor, r.lost), (&b"hello"[..], 5, 0));
        let r = s.read_rx(2, 2);
        assert_eq!((r.data.as_slice(), r.next_cursor), (&b"ll"[..], 4));
        s.push(Direction::Rx, Bytes::from_static(b"world!"), ts()); // evicts "hello"
        let r = s.read_rx(1, 100);
        assert_eq!(
            (r.data.as_slice(), r.next_cursor, r.lost),
            (&b"world!"[..], 11, 4)
        );
        assert_eq!(s.read_rx(11, 100).data, b"");
        assert_eq!(
            s.read_rx(999, 100).next_cursor,
            11,
            "cursor beyond end is clamped"
        );
    }

    proptest! {
        #[test]
        fn reading_in_pieces_equals_concatenation(
            chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..50), 0..40),
            step in 1usize..64,
        ) {
            let mut s = ChunkStore::new(usize::MAX, usize::MAX);
            let mut all = Vec::new();
            for c in &chunks {
                s.push(Direction::Rx, Bytes::from(c.clone()), ts());
                all.extend_from_slice(c);
            }
            let mut got = Vec::new();
            let mut cursor = 0;
            loop {
                let r = s.read_rx(cursor, step);
                if r.data.is_empty() { break; }
                got.extend(r.data);
                cursor = r.next_cursor;
            }
            prop_assert_eq!(got, all);
        }

        #[test]
        fn caps_hold(sizes in proptest::collection::vec(1usize..300, 1..200), max_bytes in 1usize..2000, max_chunks in 1usize..50) {
            let mut s = ChunkStore::new(max_bytes, max_chunks);
            for n in sizes {
                s.push(Direction::Rx, Bytes::from(vec![0; n]), ts());
                prop_assert!(s.len() <= max_chunks);
                prop_assert!(s.len() == 1 || s.bytes() <= max_bytes);
            }
        }

        /// `read_rx` (binary search) equals a plain walk over the retained RX chunks, with
        /// TX chunks interleaved and eviction going on.
        #[test]
        fn read_rx_matches_a_linear_walk(
            pushes in proptest::collection::vec((any::<bool>(), 0usize..40), 0..120),
            max_bytes in 1usize..600,
            cursor in 0u64..2000,
            max in 0usize..300,
        ) {
            let mut s = ChunkStore::new(max_bytes, 50);
            for (i, (rx, len)) in pushes.iter().enumerate() {
                let dir = if *rx { Direction::Rx } else { Direction::Tx };
                s.push(dir, Bytes::from(vec![i as u8; *len]), ts());
            }
            let rx: Vec<&Chunk> = s.iter().filter(|c| c.dir == Direction::Rx).collect();
            let oldest = rx.first().map_or(s.rx_total(), |c| c.offset);
            let mut pos = cursor.min(s.rx_total()).max(oldest);
            let lost = oldest.saturating_sub(cursor.min(s.rx_total()));
            let mut data = Vec::new();
            for c in rx {
                let end = c.offset + c.data.len() as u64;
                if end <= pos || data.len() >= max {
                    continue;
                }
                let skip = (pos - c.offset) as usize;
                let take = (c.data.len() - skip).min(max - data.len());
                data.extend_from_slice(&c.data[skip..skip + take]);
                pos += take as u64;
            }
            prop_assert_eq!(s.read_rx(cursor, max), RxRead { data, next_cursor: pos, lost, peers: Vec::new() });
        }
    }
}
