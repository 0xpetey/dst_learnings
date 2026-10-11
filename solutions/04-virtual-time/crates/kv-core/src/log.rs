//! The write-ahead log format.
//!
//! The node never mutates its map until the describing entry is durable.
//! Each entry is `[u32 LE length][tag][key blob][value blob?]`. Recovery
//! reads entries front to back and stops at the first one that is cut short,
//! which is what a crash between "write" and "fsync" looks like.
//!
//! Module 6 adds checksums and a sector model; until then the format is as
//! small as it can be while still surviving a torn tail.

use crate::protocol::{put_blob, DecodeError, Reader, MAX_KEY_LEN, MAX_VALUE_LEN};
use alloc::vec::Vec;

const TAG_PUT: u8 = 1;
const TAG_DELETE: u8 = 2;

/// One durable state change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

impl Entry {
    /// Append the length-prefixed encoding of this entry to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        match self {
            Entry::Put { key, value } => {
                out.push(TAG_PUT);
                put_blob(out, key);
                put_blob(out, value);
            }
            Entry::Delete { key } => {
                out.push(TAG_DELETE);
                put_blob(out, key);
            }
        }
        let len = u32::try_from(out.len() - start - 4).expect("entry fits in u32");
        out[start..start + 4].copy_from_slice(&len.to_le_bytes());
    }

    fn decode_payload(r: &mut Reader<'_>) -> Result<Entry, DecodeError> {
        match r.u8()? {
            TAG_PUT => Ok(Entry::Put {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
                value: r.blob(MAX_VALUE_LEN)?.to_vec(),
            }),
            TAG_DELETE => Ok(Entry::Delete {
                key: r.blob(MAX_KEY_LEN)?.to_vec(),
            }),
            _ => Err(DecodeError::Malformed("unknown log entry tag")),
        }
    }
}

/// What recovery found.
#[derive(Debug, PartialEq, Eq)]
pub struct Recovered {
    pub entries: Vec<Entry>,
    /// Bytes of log that were well formed. Anything past this is a torn tail
    /// and must be overwritten by the next append.
    pub valid_len: usize,
}

/// Decode the log, stopping at a torn tail.
///
/// A torn tail is *expected* (a crash mid-append). A malformed entry that is
/// not simply truncated is *not* expected with this format and is returned
/// as an error, because silently skipping it would be a silent data loss.
pub fn recover(log: &[u8]) -> Result<Recovered, DecodeError> {
    let mut entries = Vec::new();
    let mut valid_len = 0;
    loop {
        let mut r = Reader::new(&log[valid_len..]);
        let len = match r.u32() {
            Ok(len) => len as usize,
            Err(DecodeError::Incomplete) => break,
            Err(e) => return Err(e),
        };
        let payload = match r.bytes(len) {
            Ok(p) => p,
            Err(DecodeError::Incomplete) => break,
            Err(e) => return Err(e),
        };
        let mut pr = Reader::new(payload);
        let entry = Entry::decode_payload(&mut pr)?;
        if pr.remaining() != 0 {
            return Err(DecodeError::Malformed("trailing bytes in log entry"));
        }
        entries.push(entry);
        valid_len += r.position();
    }
    Ok(Recovered { entries, valid_len })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> Vec<Entry> {
        vec![
            Entry::Put {
                key: b"a".to_vec(),
                value: b"1".to_vec(),
            },
            Entry::Delete { key: b"a".to_vec() },
            Entry::Put {
                key: b"b".to_vec(),
                value: vec![9; 300],
            },
        ]
    }

    #[test]
    fn roundtrip() {
        let mut log = Vec::new();
        for e in sample() {
            e.encode(&mut log);
        }
        let r = recover(&log).unwrap();
        assert_eq!(r.entries, sample());
        assert_eq!(r.valid_len, log.len());
    }

    #[test]
    fn torn_tail_is_dropped_at_every_cut_point() {
        let mut log = Vec::new();
        for e in sample() {
            e.encode(&mut log);
        }
        let mut first_two = Vec::new();
        sample()[0].encode(&mut first_two);
        sample()[1].encode(&mut first_two);
        for cut in first_two.len()..log.len() {
            let r = recover(&log[..cut]).unwrap();
            assert_eq!(r.entries, sample()[..2], "cut at {cut}");
            assert_eq!(r.valid_len, first_two.len());
        }
    }

    #[test]
    fn empty_log_is_fine() {
        assert_eq!(
            recover(&[]).unwrap(),
            Recovered {
                entries: vec![],
                valid_len: 0
            }
        );
    }

    #[test]
    fn corruption_is_an_error_not_a_skip() {
        let mut log = Vec::new();
        sample()[0].encode(&mut log);
        log[4] = 0xee; // bad tag
        assert!(recover(&log).is_err());
    }
}
