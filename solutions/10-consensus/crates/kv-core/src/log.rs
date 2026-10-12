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
//! the rest because its disk model needs every field. Module 9 put the
//! view a record was prepared in into the header. Module 10 adds the
//! checksum of the record before it, so that a log is a hash chain: a
//! record vouches for its whole prefix, and a replica that is offered a
//! record for the end of its log can tell whether the record belongs
//! there without asking anyone.

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
pub const HEADER_LEN: usize = 28;
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

impl Entry {
    pub fn key(&self) -> &[u8] {
        match self {
            Entry::Put { key, .. } | Entry::Delete { key } => key,
        }
    }
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

    /// Append this record, numbered `seq`, to `out` as whole sectors, in
    /// view 0 and chained to `parent`, the checksum of the record before
    /// it. A single node has one view. Returns the record's checksum, the
    /// parent of the next one.
    pub fn encode(&self, seq: u64, parent: u32, out: &mut Vec<u8>) -> u32 {
        self.encode_in_view(seq, 0, parent, out)
    }

    /// Append this record, numbered `seq` and prepared by the primary of
    /// `view`, to `out` as whole sectors. Module 9 put the view in the
    /// header: two records can carry one sequence number when a primary
    /// loses a write before anyone saw it and a later primary numbers a
    /// new write the same. The view says which one the cluster kept.
    /// `parent` is the checksum of the record before this one, zero for
    /// the first, and the returned checksum is the parent of the next.
    pub fn encode_in_view(&self, seq: u64, view: u32, parent: u32, out: &mut Vec<u8>) -> u32 {
        let start = out.len();
        put_u32(out, MAGIC);
        put_u32(out, 0); // payload length, patched below
        put_u64(out, seq);
        put_u32(out, 0); // checksum, patched below
        put_u32(out, view);
        put_u32(out, parent);
        debug_assert_eq!(out.len() - start, HEADER_LEN);
        self.encode_payload(out);
        let payload_len = out.len() - start - HEADER_LEN;
        assert!(payload_len <= MAX_PAYLOAD_LEN);
        out.resize(start + record_len(payload_len), 0);
        out[start + 4..start + 8].copy_from_slice(&(payload_len as u32).to_le_bytes());
        let crc = checksum(&out[start..]);
        out[start + 16..start + 20].copy_from_slice(&crc.to_le_bytes());
        crc
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
    /// The view each record was prepared in, parallel to `records`.
    pub views: Vec<u32>,
    /// Each record's checksum, parallel to `records`. The last one is the
    /// parent of the next record.
    pub checksums: Vec<u32>,
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
        header: Header,
        len: usize,
        record: Record,
    },
}

/// The fields of a record's header that name it: its number, the view it
/// was prepared in, its own checksum and the checksum of the record
/// before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub seq: u64,
    pub view: u32,
    pub checksum: u32,
    pub parent: u32,
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
    let view = r.u32().expect("header has view");
    let parent = r.u32().expect("header has parent");
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
        Ok(record) => Probe::Valid {
            header: Header {
                seq,
                view,
                checksum: crc,
                parent,
            },
            len,
            record,
        },
        Err(_) => Probe::Invalid,
    }
}

/// The header of `bytes`, if they are exactly one valid record.
pub fn record_header(bytes: &[u8]) -> Option<Header> {
    match probe(bytes, 0) {
        Probe::Valid { header, len, .. } if len == bytes.len() => Some(header),
        _ => None,
    }
}

/// Decode the log. Never fails: a problem is reported in `tail`, and the
/// caller decides what it means. A node that finds `Tail::Damaged` must
/// refuse to serve.
pub fn recover(log: &[u8]) -> Recovered {
    let mut records = Vec::new();
    let mut views = Vec::new();
    let mut checksums = Vec::new();
    let mut pos = 0;
    let mut seq = 1;
    let mut parent = 0;
    loop {
        match probe(log, pos) {
            Probe::Valid { header, len, record } if header.seq == seq && header.parent == parent => {
                records.push(record);
                views.push(header.view);
                checksums.push(header.checksum);
                parent = header.checksum;
                pos += len;
                seq += 1;
            }
            Probe::Valid { header, .. } => {
                // A well-formed record where a different one belongs: a
                // write that landed in the wrong place, a stale record
                // from before a torn tail was overwritten, or a record
                // with the right number that was chained to a record this
                // log no longer has.
                return finish(records, views, checksums, pos, seq, Some((pos, header.seq)), log);
            }
            Probe::Invalid => {
                let next = scan_forward(log, pos + SECTOR_SIZE);
                return finish(records, views, checksums, pos, seq, next, log);
            }
            Probe::Short => return finish(records, views, checksums, pos, seq, None, log),
        }
    }
}

/// Every valid record at or after `from`, at any offset, as `(header,
/// bytes)` in offset order. A replica whose log has a hole uses this to
/// keep what lies beyond the hole: those records are real, only the one
/// before them is missing, and a peer can supply it. A stray copy of a
/// record that the cluster has since replaced is real too, and only its
/// chain tells it apart: it is written back only once the record it
/// names as its parent is in place.
pub fn salvage(log: &[u8], from: usize) -> Vec<(Header, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pos = from.next_multiple_of(SECTOR_SIZE);
    while pos < log.len() {
        match probe(log, pos) {
            Probe::Valid { header, len, .. } => {
                found.push((header, log[pos..pos + len].to_vec()));
                pos += len;
            }
            _ => pos += SECTOR_SIZE,
        }
    }
    found
}

/// The first valid record at or after `from`, at any sequence number.
fn scan_forward(log: &[u8], from: usize) -> Option<(usize, u64)> {
    let mut pos = from;
    while pos < log.len() {
        if let Probe::Valid { header, .. } = probe(log, pos) {
            return Some((pos, header.seq));
        }
        pos += SECTOR_SIZE;
    }
    None
}

fn finish(
    records: Vec<Record>,
    views: Vec<u32>,
    checksums: Vec<u32>,
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
        views,
        checksums,
        valid_len,
        tail,
    }
}
