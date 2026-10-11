//! The write-ahead log format: sector-aligned, checksummed, numbered records.
//!
//! ```text
//!   sector 0        sector 1        sector 2        sector 3
//!   ┌───────────────┬───────────────┬───────────────┬───────────────┐
//!   │ hdr │ payload │ hdr │ payload │ ... payload   │ hdr │ payload │
//!   │ seq=1         │ seq=2 (two sectors)           │ seq=3         │
//!   └───────────────┴───────────────┴───────────────┴───────────────┘
//! ```
//!
//! Every record starts on a sector boundary and is zero padded to a whole
//! number of sectors. The header carries a magic number, the payload length,
//! the record's sequence number and a CRC-32C over everything else in the
//! record, padding included. A disk can tear a record across its sectors,
//! flip a bit in it, write it to the wrong place, or drop it after saying
//! it was written. Each of those leaves a record whose checksum does not
//! match, or whose sequence number is not the one expected at that offset.
//!
//! Recovery reads records front to back. It stops at the first record that
//! does not check out, and then asks one more question: is there a valid
//! record anywhere after it? If not, the bad bytes are a torn write at the
//! tail, which a crash in the middle of an append produces and which is
//! safe to overwrite. If there is, a record that was once written is now
//! missing or broken in the middle of the log. That is damage, and a node
//! that serves requests on top of it would be serving a lie.
//!
//! Module 2's format was a length prefix and nothing else. Module 6 adds
//! the rest because its disk model needs every field.

use crate::checksum::crc32c_update;
use crate::protocol::{
    put_blob, put_u32, put_u64, ClientId, DecodeError, Reader, RequestId, MAX_KEY_LEN, MAX_VALUE_LEN,
};
use alloc::vec::Vec;
use core::fmt;

/// The unit of atomic I/O. A write that spans several sectors can land
/// in part.
pub const SECTOR_SIZE: usize = 512;
/// Bytes of header at the start of every record.
pub const HEADER_LEN: usize = 24;
/// Largest payload a record can carry: tag, client, request id, key, value.
pub const MAX_PAYLOAD_LEN: usize = 1 + 8 + 8 + 4 + MAX_KEY_LEN + 4 + MAX_VALUE_LEN;

/// `KVL1`, little endian.
const MAGIC: u32 = u32::from_le_bytes(*b"KVL1");
const TAG_PUT: u8 = 1;
const TAG_DELETE: u8 = 2;

/// One durable state change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// One log record: the change and the request that asked for it. The
/// request identity is in the log so that a restarted node can still tell
/// a retry from a new request. Module 6 found out why the hard way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub client: ClientId,
    pub request_id: RequestId,
    pub entry: Entry,
}

/// Round `len` up to whole sectors.
pub const fn sectors(len: usize) -> usize {
    len.div_ceil(SECTOR_SIZE)
}

/// Bytes a record with `payload_len` bytes of payload occupies on disk.
pub const fn record_len(payload_len: usize) -> usize {
    sectors(HEADER_LEN + payload_len) * SECTOR_SIZE
}

impl Record {
    fn encode_payload(&self, out: &mut Vec<u8>) {
        match &self.entry {
            Entry::Put { key, value } => {
                out.push(TAG_PUT);
                put_u64(out, self.client);
                put_u64(out, self.request_id);
                put_blob(out, key);
                put_blob(out, value);
            }
            Entry::Delete { key } => {
                out.push(TAG_DELETE);
                put_u64(out, self.client);
                put_u64(out, self.request_id);
                put_blob(out, key);
            }
        }
    }

    fn decode_payload(payload: &[u8]) -> Result<Record, DecodeError> {
        let mut r = Reader::new(payload);
        let tag = r.u8()?;
        let client = r.u64()?;
        let request_id = r.u64()?;
        let entry = match tag {
            TAG_PUT => Entry::Put {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
                value: r.blob(MAX_VALUE_LEN)?.to_vec(),
            },
            TAG_DELETE => Entry::Delete {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
            },
            _ => return Err(DecodeError::Malformed("unknown log entry tag")),
        };
        if r.remaining() != 0 {
            return Err(DecodeError::Malformed("trailing bytes in log entry"));
        }
        Ok(Record {
            client,
            request_id,
            entry,
        })
    }

    /// Append this record, numbered `seq`, to `out` as whole sectors.
    pub fn encode(&self, seq: u64, out: &mut Vec<u8>) {
        let start = out.len();
        put_u32(out, MAGIC);
        put_u32(out, 0); // payload length, patched below
        put_u64(out, seq);
        put_u32(out, 0); // checksum, patched below
        put_u32(out, 0); // reserved
        debug_assert_eq!(out.len() - start, HEADER_LEN);
        self.encode_payload(out);
        let payload_len = out.len() - start - HEADER_LEN;
        assert!(payload_len <= MAX_PAYLOAD_LEN);
        out.resize(start + record_len(payload_len), 0);
        out[start + 4..start + 8].copy_from_slice(&(payload_len as u32).to_le_bytes());
        let crc = checksum(&out[start..]);
        out[start + 16..start + 20].copy_from_slice(&crc.to_le_bytes());
    }

    /// Bytes this record occupies on disk.
    pub fn encoded_len(&self) -> usize {
        let mut payload = Vec::new();
        self.encode_payload(&mut payload);
        record_len(payload.len())
    }
}

/// CRC-32C over a whole record with the checksum field skipped.
fn checksum(record: &[u8]) -> u32 {
    let crc = crc32c_update(0, &record[..16]);
    crc32c_update(crc, &record[20..])
}

/// A record that was once in the log and is now unreadable, with a valid
/// record after it to prove it was once there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    /// Offset of the record that failed to check out.
    pub at: usize,
    /// The sequence number expected there.
    pub seq: u64,
    /// Offset of the first valid record after it.
    pub next_valid_at: usize,
    /// Its sequence number. Equal to `seq` for a record that landed in the
    /// wrong place.
    pub next_valid_seq: u64,
}

impl fmt::Display for Damage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "record {} at offset {} is unreadable, record {} is at offset {}",
            self.seq, self.at, self.next_valid_seq, self.next_valid_at
        )
    }
}

/// What the bytes after the last valid record look like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tail {
    /// Nothing but zeros, or nothing at all.
    Clean,
    /// Bytes at `at` that are not a record, with no record after them. A
    /// crash in the middle of an append leaves this. The next append
    /// overwrites it.
    Torn { at: usize },
    /// A record is missing or broken, and records after it survive. An
    /// acknowledged write is gone. The node must not start.
    Damaged(Damage),
}

impl fmt::Display for Tail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tail::Clean => write!(f, "clean"),
            Tail::Torn { at } => write!(f, "torn at offset {at}"),
            Tail::Damaged(d) => write!(f, "damaged: {d}"),
        }
    }
}

/// What recovery found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    /// Every record up to the first problem, in order. Record `i` has
    /// sequence number `i + 1`.
    pub records: Vec<Record>,
    /// Bytes of log that were well formed. The next append goes here.
    pub valid_len: usize,
    pub tail: Tail,
}

/// One record's worth of bytes, examined.
enum Probe {
    /// Fewer than `HEADER_LEN` bytes remain.
    Short,
    /// Not a record: bad magic, bad length, bad checksum or bad payload.
    Invalid,
    Valid {
        seq: u64,
        len: usize,
        record: Record,
    },
}

fn probe(log: &[u8], pos: usize) -> Probe {
    let Some(header) = log.get(pos..pos + HEADER_LEN) else {
        return Probe::Short;
    };
    let mut r = Reader::new(header);
    let magic = r.u32().expect("header has magic");
    let payload_len = r.u32().expect("header has length") as usize;
    let seq = r.u64().expect("header has seq");
    let crc = r.u32().expect("header has checksum");
    if magic != MAGIC || payload_len > MAX_PAYLOAD_LEN {
        return Probe::Invalid;
    }
    let len = record_len(payload_len);
    let Some(record) = log.get(pos..pos + len) else {
        return Probe::Invalid;
    };
    if checksum(record) != crc {
        return Probe::Invalid;
    }
    match Record::decode_payload(&record[HEADER_LEN..HEADER_LEN + payload_len]) {
        Ok(record) => Probe::Valid { seq, len, record },
        Err(_) => Probe::Invalid,
    }
}

/// Decode the log. Never fails: a problem is reported in `tail`, and the
/// caller decides what it means. A node that finds `Tail::Damaged` must
/// refuse to serve.
pub fn recover(log: &[u8]) -> Recovered {
    let mut records = Vec::new();
    let mut pos = 0;
    let mut seq = 1;
    loop {
        match probe(log, pos) {
            Probe::Valid {
                seq: found,
                len,
                record,
            } if found == seq => {
                records.push(record);
                pos += len;
                seq += 1;
            }
            Probe::Valid { seq: found, .. } => {
                // A well-formed record where a different one belongs: a
                // write that landed in the wrong place, or a stale record
                // from before a torn tail was overwritten.
                return finish(records, pos, seq, Some((pos, found)), log);
            }
            Probe::Invalid => {
                let next = scan_forward(log, pos + SECTOR_SIZE);
                return finish(records, pos, seq, next, log);
            }
            Probe::Short => return finish(records, pos, seq, None, log),
        }
    }
}

/// The first valid record at or after `from`, at any sequence number.
fn scan_forward(log: &[u8], from: usize) -> Option<(usize, u64)> {
    let mut pos = from;
    while pos < log.len() {
        if let Probe::Valid { seq, .. } = probe(log, pos) {
            return Some((pos, seq));
        }
        pos += SECTOR_SIZE;
    }
    None
}

fn finish(
    records: Vec<Record>,
    valid_len: usize,
    seq: u64,
    next: Option<(usize, u64)>,
    log: &[u8],
) -> Recovered {
    let tail = match next {
        Some((next_valid_at, next_valid_seq)) => Tail::Damaged(Damage {
            at: valid_len,
            seq,
            next_valid_at,
            next_valid_seq,
        }),
        None if log[valid_len..].iter().all(|&b| b == 0) => Tail::Clean,
        None => Tail::Torn { at: valid_len },
    };
    Recovered {
        records,
        valid_len,
        tail,
    }
}
