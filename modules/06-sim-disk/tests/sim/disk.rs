//! Acceptance tests for the disk model. The model decides fates; these
//! tests check the decisions without a scheduler in the way.

use determinism::Prng;
use sim::{Chance, Completion, Disk, DiskConfig, Duration, Instant, WriteFate};

const S: usize = 512;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(n: u64) -> Instant {
    Instant::ZERO + ms(n)
}
fn reliable() -> DiskConfig {
    DiskConfig::reliable(ms(1), ms(5))
}
fn prng() -> Prng {
    Prng::from_seed(1)
}
fn sector(fill: u8) -> Vec<u8> {
    vec![fill; S]
}

/// Bits that differ between two equal-length slices.
fn bits_differing(a: &[u8], b: &[u8]) -> u32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

#[test]
fn a_reliable_disk_writes_and_reads_back_after_the_latency() {
    let mut disk = Disk::new(reliable());
    let mut p = prng();
    assert_eq!(disk.submit_write(at(0), 1, 0, &sector(1), &mut p), at(5));
    assert_eq!(disk.submit_write(at(0), 2, S as u64, &sector(2), &mut p), at(5));
    assert_eq!(disk.pending(), 2);
    assert!(matches!(
        disk.complete_next(&mut p),
        Completion::Write {
            op: 1,
            fate: WriteFate::Landed,
            ..
        }
    ));
    assert!(matches!(
        disk.complete_next(&mut p),
        Completion::Write { op: 2, .. }
    ));
    // A read submitted while the disk is busy completes after the queue.
    assert_eq!(
        disk.submit_write(at(10), 3, 2 * S as u64, &sector(3), &mut p),
        at(15)
    );
    assert_eq!(disk.submit_read(at(10), 4, &mut p), at(15));
    disk.complete_next(&mut p);
    let Completion::Read { op, data, faults } = disk.complete_next(&mut p) else {
        panic!("expected the read")
    };
    assert_eq!(op, 4);
    assert!(faults.is_empty());
    assert_eq!(data, [sector(1), sector(2), sector(3)].concat());
    assert_eq!(data, disk.medium());
    let s = disk.stats();
    assert_eq!((s.reads, s.writes, s.landed), (1, 3, 3));
    assert_eq!(
        (s.lost, s.torn, s.misdirected, s.corrupted, s.read_faults),
        (0, 0, 0, 0, 0)
    );
}

#[test]
fn the_medium_is_whole_sectors() {
    let mut disk = Disk::with_medium(reliable(), vec![7; 100]);
    assert_eq!(disk.medium().len(), S);
    assert_eq!(&disk.medium()[..100], &[7; 100][..]);
    assert!(disk.medium()[100..].iter().all(|&b| b == 0));
    let mut p = prng();
    disk.submit_write(at(0), 1, S as u64 + 10, &[1, 2, 3], &mut p);
    disk.complete_next(&mut p);
    assert_eq!(disk.medium().len(), 2 * S);
}

#[test]
fn a_lost_write_is_reported_complete_and_never_lands() {
    let mut disk = Disk::new(DiskConfig {
        lost_write: Chance::ALWAYS,
        ..reliable()
    });
    let mut p = prng();
    disk.submit_write(at(0), 1, 0, &sector(9), &mut p);
    assert!(matches!(
        disk.complete_next(&mut p),
        Completion::Write {
            fate: WriteFate::Lost,
            ..
        }
    ));
    assert_eq!(disk.medium().len(), 0, "nothing reached the medium");
    assert_eq!(disk.stats().lost, 1);
    assert_eq!(disk.stats().landed, 0);
}

#[test]
fn lose_next_write_loses_exactly_one() {
    let mut disk = Disk::new(reliable());
    let mut p = prng();
    disk.lose_next_write();
    disk.submit_write(at(0), 1, 0, &sector(1), &mut p);
    disk.submit_write(at(0), 2, S as u64, &sector(2), &mut p);
    assert!(matches!(
        disk.complete_next(&mut p),
        Completion::Write {
            fate: WriteFate::Lost,
            ..
        }
    ));
    assert!(matches!(
        disk.complete_next(&mut p),
        Completion::Write {
            fate: WriteFate::Landed,
            ..
        }
    ));
    assert_eq!(disk.medium(), [sector(0), sector(2)].concat());
}

#[test]
fn a_crash_on_a_reliable_disk_lands_what_was_in_flight() {
    // A process crash is not a power loss. The device finishes the writes
    // it was given, and reads in flight go nowhere.
    let mut disk = Disk::new(reliable());
    let mut p = prng();
    disk.submit_read(at(0), 0, &mut p);
    disk.submit_write(at(0), 1, 0, &sector(1), &mut p);
    disk.submit_write(at(0), 2, S as u64, &sector(2), &mut p);
    let report = disk.crash(&mut p);
    assert_eq!(
        (report.landed, report.torn, report.lost, report.reads_dropped),
        (2, 0, 0, 1)
    );
    assert_eq!(disk.pending(), 0);
    assert_eq!(disk.medium(), [sector(1), sector(2)].concat());
    assert_eq!(disk.stats().crashes, 1);
    // The disk is free again: a write submitted now completes at its own latency.
    assert_eq!(disk.submit_write(at(100), 3, 0, &sector(3), &mut p), at(105));
}

#[test]
fn a_torn_write_keeps_a_strict_prefix_of_sectors_and_loses_the_rest() {
    let mut p = prng();
    let mut seen_partial = false;
    for _ in 0..50 {
        let mut disk = Disk::new(DiskConfig {
            torn_write: Chance::ALWAYS,
            ..reliable()
        });
        let write: Vec<u8> = [sector(1), sector(2), sector(3)].concat();
        disk.submit_write(at(0), 1, 0, &write, &mut p);
        disk.submit_write(at(0), 2, 3 * S as u64, &sector(4), &mut p);
        let report = disk.crash(&mut p);
        assert_eq!((report.torn, report.lost, report.landed), (1, 1, 0));
        let medium = disk.medium();
        assert!(medium.len() <= 3 * S, "the second write never lands");
        let landed = medium.len() / S;
        assert!(landed < 3, "a torn write never lands in full");
        assert_eq!(medium, &write[..landed * S]);
        seen_partial |= landed > 0;
    }
    assert!(seen_partial, "some tears keep a prefix");
}

#[test]
fn a_corrupt_write_flips_one_bit_and_remembers_the_sector() {
    let mut disk = Disk::new(DiskConfig {
        corrupt_write: Chance::ALWAYS,
        ..reliable()
    });
    let mut p = prng();
    let write = [sector(0x55), sector(0xaa)].concat();
    disk.submit_write(at(0), 1, 0, &write, &mut p);
    let Completion::Write { faults, .. } = disk.complete_next(&mut p) else {
        panic!()
    };
    assert_eq!(faults, vec![0, 1]);
    assert_eq!(disk.faults().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(bits_differing(disk.medium(), &write), 2, "one bit per sector");
    assert_eq!(disk.stats().corrupted, 2);

    // Rewriting a sector cleans it.
    disk.set_config(reliable());
    disk.submit_write(at(10), 2, S as u64, &sector(0xaa), &mut p);
    disk.complete_next(&mut p);
    assert_eq!(disk.faults().collect::<Vec<_>>(), vec![0]);
    assert_eq!(&disk.medium()[S..], &sector(0xaa)[..]);
}

#[test]
fn a_corrupt_read_is_wrong_this_time_only() {
    let mut disk = Disk::with_medium(
        DiskConfig {
            corrupt_read: Chance::ALWAYS,
            ..reliable()
        },
        [sector(1), sector(2)].concat(),
    );
    let mut p = prng();
    disk.submit_read(at(0), 0, &mut p);
    let Completion::Read { data, faults, .. } = disk.complete_next(&mut p) else {
        panic!()
    };
    assert_eq!(faults, vec![0, 1]);
    assert_eq!(bits_differing(&data, disk.medium()), 2);
    assert!(disk.faults().next().is_none(), "the medium is untouched");
    assert_eq!(disk.stats().read_faults, 2);

    disk.set_config(reliable());
    disk.submit_read(at(1), 1, &mut p);
    let Completion::Read { data, faults, .. } = disk.complete_next(&mut p) else {
        panic!()
    };
    assert!(faults.is_empty());
    assert_eq!(data, disk.medium());
}

#[test]
fn a_misdirected_write_lands_whole_at_another_aligned_offset() {
    let mut p = prng();
    for _ in 0..20 {
        let mut disk = Disk::with_medium(
            DiskConfig {
                misdirect_write: Chance::ALWAYS,
                ..reliable()
            },
            vec![0; 4 * S],
        );
        disk.submit_write(at(0), 1, S as u64, &sector(7), &mut p);
        let Completion::Write { fate, .. } = disk.complete_next(&mut p) else {
            panic!()
        };
        let WriteFate::Misdirected { to } = fate else {
            panic!("{fate}")
        };
        assert_ne!(to, S as u64);
        assert_eq!(to % S as u64, 0);
        let to = to as usize;
        assert_eq!(&disk.medium()[to..to + S], &sector(7)[..]);
        assert!(
            disk.medium()[S..2 * S].iter().all(|&b| b == 0),
            "the right place is untouched"
        );
        assert_eq!(disk.stats().misdirected, 1);
    }
}

#[test]
fn corrupt_sector_is_a_test_knob() {
    let mut disk = Disk::with_medium(reliable(), vec![0; 2 * S]);
    let mut p = prng();
    disk.corrupt_sector(1, &mut p);
    assert_eq!(disk.faults().collect::<Vec<_>>(), vec![1]);
    assert_eq!(disk.medium()[..S].iter().filter(|&&b| b != 0).count(), 0);
    assert_eq!(bits_differing(&disk.medium()[S..], &sector(0)), 1);
}

#[test]
fn a_random_config_is_a_function_of_the_seed() {
    let a = DiskConfig::random(&mut Prng::from_seed(3));
    let b = DiskConfig::random(&mut Prng::from_seed(3));
    assert_eq!(a, b);
    a.validate();
    assert!(!a.is_reliable() || DiskConfig::random(&mut Prng::from_seed(4)) != a);
    assert!(a.write_latency_max >= a.write_latency_min);
    assert!(reliable().is_reliable());
}

#[test]
#[should_panic(expected = "read_latency_min")]
fn an_inverted_latency_range_is_rejected() {
    Disk::new(DiskConfig {
        read_latency_min: ms(2),
        read_latency_max: ms(1),
        ..reliable()
    });
}

#[test]
#[should_panic(expected = "nothing pending")]
fn completing_an_empty_queue_is_a_bug() {
    Disk::new(reliable()).complete_next(&mut prng());
}
