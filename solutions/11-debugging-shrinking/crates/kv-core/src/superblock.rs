//! The superblock: what a replica must remember across a restart besides
//! its log.
//!
//! ```text
//!   sector 0        sector 1        sector 2 ...
//!   ┌───────────────┬───────────────┬──────────────────────────────
//!   │ superblock A  │ superblock B  │ the log, as log.rs lays it out
//!   └───────────────┴───────────────┴──────────────────────────────
//! ```
//!
//! Modules 6 to 9 kept nothing but the log, and Module 6 recorded the
//! consequence: a replica that restarts does not know how long its log
//! was, or which view it had agreed to. Seed 562 of Module 10 is the
//! second half of that bill. A replica that had voted for a view change
//! crashed, forgot the vote, and rejoined the old view, and two views
//! each had a majority. Raft writes the term to disk before it votes.
//! Viewstamped Replication keeps the view in memory and pays with a
//! recovery protocol. This is Raft's answer.
//!
//! Four copies, written in turn. A write that tears leaves the others
//! whole. The copy with the highest `seq` that checks out is the truth.
//! The fields: the highest `view` this replica has agreed to, the last
//! view it held a log the primary of that view vouched for, and how long
//! that log was. A restarted replica whose log is shorter than that has
//! lost records, and claims the view of its last record instead.

use crate::checksum::crc32c;
use crate::log::SECTOR_SIZE;
use crate::protocol::{put_u32, put_u64, Reader};
use alloc::vec::Vec;

/// `KVSB`, little endian.
const MAGIC: u32 = u32::from_le_bytes(*b"KVSB");
/// Copies of the superblock, one sector each. Each write goes to the
/// next copy, so a write that tears or a sector that rots costs one copy
/// of four, and the newest one left is at most one write old.
pub const COPIES: usize = 4;
/// Where the log begins.
pub const LOG_OFFSET: usize = COPIES * SECTOR_SIZE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Superblock {
    /// One more with every write. The newer copy wins.
    pub seq: u64,
    /// The highest view this replica has agreed to. It never takes part
    /// in a lower one again.
    pub view: u64,
    /// The last view this replica was normal in.
    pub log_view: u64,
    /// Records in the log when `log_view` was claimed.
    pub log_view_len: u64,
}

impl Superblock {
    /// The sector this copy goes to: the copies alternate.
    pub fn offset(&self) -> usize {
        (self.seq % COPIES as u64) as usize * SECTOR_SIZE
    }

    /// One sector: the fields, then a CRC-32C over them, then zeros.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SECTOR_SIZE);
        put_u32(&mut out, MAGIC);
        put_u64(&mut out, self.seq);
        put_u64(&mut out, self.view);
        put_u64(&mut out, self.log_view);
        put_u64(&mut out, self.log_view_len);
        let crc = crc32c(&out);
        put_u32(&mut out, crc);
        out.resize(SECTOR_SIZE, 0);
        out
    }

    /// The fields of one sector, if it is a superblock that checks out.
    pub fn decode(sector: &[u8]) -> Option<Superblock> {
        if sector.len() < 40 {
            return None;
        }
        let mut r = Reader::new(sector);
        let magic = r.u32().ok()?;
        let seq = r.u64().ok()?;
        let view = r.u64().ok()?;
        let log_view = r.u64().ok()?;
        let log_view_len = r.u64().ok()?;
        let crc = r.u32().ok()?;
        if magic != MAGIC || crc32c(&sector[..36]) != crc {
            return None;
        }
        Some(Superblock {
            seq,
            view,
            log_view,
            log_view_len,
        })
    }
}

/// The newest superblock on `disk`, if either copy checks out.
pub fn read(disk: &[u8]) -> Option<Superblock> {
    (0..COPIES)
        .filter_map(|i| disk.get(i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE))
        .filter_map(Superblock::decode)
        .max_by_key(|sb| sb.seq)
}

/// The log's bytes on `disk`: everything past the superblock.
pub fn log_bytes(disk: &[u8]) -> &[u8] {
    &disk[LOG_OFFSET.min(disk.len())..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn roundtrip_and_newest_copy_wins() {
        let a = Superblock {
            seq: 1,
            view: 3,
            log_view: 2,
            log_view_len: 10,
        };
        let b = Superblock { seq: 2, view: 4, ..a };
        assert_eq!(Superblock::decode(&a.encode()), Some(a));
        let mut disk = vec![0; LOG_OFFSET + 7];
        disk[b.offset()..b.offset() + SECTOR_SIZE].copy_from_slice(&b.encode());
        disk[a.offset()..a.offset() + SECTOR_SIZE].copy_from_slice(&a.encode());
        assert_eq!(read(&disk), Some(b));
        assert_eq!(log_bytes(&disk).len(), 7);
        // A torn copy loses, a flipped bit is not a superblock.
        disk[b.offset() + 5] ^= 1;
        assert_eq!(read(&disk), Some(a));
        assert_eq!(read(&[0; 100]), None);
        assert_eq!(log_bytes(&[0; 100]).len(), 0);
    }
}
