//! Acceptance tests for the log format: whole sectors, a checksum on
//! every record, a sequence number in every header, and a recovery that
//! tells a torn tail from a hole.

use kv_core::log::{record_len, recover, Damage, Entry, Record, Tail, HEADER_LEN, SECTOR_SIZE};

fn rec(request_id: u64, entry: Entry) -> Record {
    Record {
        client: 7,
        request_id,
        entry,
    }
}

fn sample() -> Vec<Record> {
    vec![
        rec(
            1,
            Entry::Put {
                key: b"a".to_vec(),
                value: b"1".to_vec(),
            },
        ),
        rec(2, Entry::Delete { key: b"a".to_vec() }),
        rec(
            3,
            Entry::Put {
                key: b"b".to_vec(),
                value: vec![9; 700],
            },
        ),
    ]
}

fn encode_all(records: &[Record]) -> Vec<u8> {
    let mut log = Vec::new();
    for (i, r) in records.iter().enumerate() {
        r.encode(i as u64 + 1, &mut log);
    }
    log
}

#[test]
fn records_are_whole_sectors() {
    let log = encode_all(&sample());
    // One sector, one sector, two sectors (700 bytes of value).
    assert_eq!(log.len(), 4 * SECTOR_SIZE);
    assert_eq!(sample()[2].encoded_len(), 2 * SECTOR_SIZE);
    assert_eq!(record_len(0), SECTOR_SIZE);
    assert_eq!(record_len(SECTOR_SIZE - HEADER_LEN), SECTOR_SIZE);
    assert_eq!(record_len(SECTOR_SIZE - HEADER_LEN + 1), 2 * SECTOR_SIZE);
}

#[test]
fn roundtrip() {
    let log = encode_all(&sample());
    let r = recover(&log);
    assert_eq!(r.records, sample());
    assert_eq!(r.valid_len, log.len());
    assert_eq!(r.tail, Tail::Clean);
}

#[test]
fn empty_log_is_clean() {
    let r = recover(&[]);
    assert_eq!(r.records, vec![]);
    assert_eq!((r.valid_len, r.tail), (0, Tail::Clean));
    let zeros = vec![0; 3 * SECTOR_SIZE];
    assert_eq!(recover(&zeros).tail, Tail::Clean);
}

#[test]
fn a_torn_tail_is_cut_at_every_cut_point() {
    let log = encode_all(&sample());
    let first_two = encode_all(&sample()[..2]);
    for cut in first_two.len() + 1..log.len() {
        let r = recover(&log[..cut]);
        assert_eq!(r.records, sample()[..2], "cut at {cut}");
        assert_eq!(r.valid_len, first_two.len());
        assert_eq!(r.tail, Tail::Torn { at: first_two.len() }, "cut at {cut}");
    }
}

#[test]
fn every_bit_flip_in_the_valid_region_is_caught() {
    let log = encode_all(&sample());
    let third_starts = 2 * SECTOR_SIZE;
    for byte in 0..log.len() {
        for bit in 0..8 {
            let mut bad = log.clone();
            bad[byte] ^= 1 << bit;
            let r = recover(&bad);
            let in_record = byte / SECTOR_SIZE;
            let record = if in_record >= 2 { 2 } else { in_record };
            // Records before the flipped one survive, the flipped one
            // never decodes, and the ones after it prove the damage.
            assert_eq!(r.records, sample()[..record], "byte {byte} bit {bit}");
            if record == 2 {
                assert_eq!(r.tail, Tail::Torn { at: third_starts }, "byte {byte} bit {bit}");
            } else {
                let next_valid_at = (record + 1) * SECTOR_SIZE;
                assert_eq!(
                    r.tail,
                    Tail::Damaged(Damage {
                        at: record * SECTOR_SIZE,
                        seq: record as u64 + 1,
                        next_valid_at,
                        next_valid_seq: record as u64 + 2,
                    }),
                    "byte {byte} bit {bit}"
                );
            }
        }
    }
}

#[test]
fn a_lost_record_in_the_middle_is_damage() {
    let mut log = encode_all(&sample());
    // Record 2 was acknowledged and never landed: its sector is zeros.
    log[SECTOR_SIZE..2 * SECTOR_SIZE].fill(0);
    let r = recover(&log);
    assert_eq!(r.records, sample()[..1]);
    assert_eq!(
        r.tail,
        Tail::Damaged(Damage {
            at: SECTOR_SIZE,
            seq: 2,
            next_valid_at: 2 * SECTOR_SIZE,
            next_valid_seq: 3,
        })
    );
}

#[test]
fn a_lost_record_at_the_tail_is_invisible() {
    let mut log = encode_all(&sample());
    log.truncate(2 * SECTOR_SIZE);
    log.resize(4 * SECTOR_SIZE, 0);
    let r = recover(&log);
    assert_eq!(r.records, sample()[..2]);
    assert_eq!(r.tail, Tail::Clean, "a lost tail looks like no write at all");
}

#[test]
fn a_record_in_the_wrong_place_is_damage() {
    let log = encode_all(&sample());
    let mut bad = log.clone();
    // Record 3 landed where record 2 belongs.
    let third = log[2 * SECTOR_SIZE..].to_vec();
    bad[SECTOR_SIZE..SECTOR_SIZE + third.len()].copy_from_slice(&third);
    let r = recover(&bad);
    assert_eq!(r.records, sample()[..1]);
    assert_eq!(
        r.tail,
        Tail::Damaged(Damage {
            at: SECTOR_SIZE,
            seq: 2,
            next_valid_at: SECTOR_SIZE,
            next_valid_seq: 3,
        })
    );
}

#[test]
fn a_stale_record_past_a_torn_tail_is_still_damage() {
    // Records 1, 2 valid; record 3's header sector torn; record 4
    // landed beyond it. Nothing can say whether 3 was acknowledged, so
    // recovery reports damage and leaves the decision to the caller.
    let four = {
        let mut s = sample();
        s.push(rec(4, Entry::Delete { key: b"b".to_vec() }));
        s
    };
    let mut log = encode_all(&four);
    log[2 * SECTOR_SIZE + 5] ^= 0xff;
    let r = recover(&log);
    assert_eq!(r.records, four[..2]);
    assert!(matches!(r.tail, Tail::Damaged(d) if d.next_valid_seq == 4));
}

#[test]
fn record_identity_survives() {
    let log = encode_all(&sample());
    let r = recover(&log);
    assert_eq!(r.records[1].client, 7);
    assert_eq!(r.records[1].request_id, 2);
}
