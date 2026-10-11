//! Module 6 acceptance tests: hosts crash and restart over a disk that
//! tears, lies, rots and misdirects.
//!
//! Three things must hold whatever the faults:
//!
//! 1. An acknowledged write survives a crash and a restart, unless the
//!    disk itself lost it after saying it was written.
//! 2. The node never serves a record the disk corrupted. A flipped bit is
//!    caught, and a hole in the middle of the log halts the node.
//! 3. A retry across a restart is applied once.

use determinism::{check_determinism, Prng};
use kv_core::client::Config;
use kv_core::heartbeat::Liveness::{self, Down, Up};
use kv_core::log::{Entry, Tail, SECTOR_SIZE};
use kv_core::{Duration, NodeId, Request, Response};
use kv_env::fake::{Addr, CrashSchedule, PartitionSchedule, World, WorldConfig};
use sim::{Chance, Disk, DiskConfig, NetConfig};
use std::collections::{BTreeMap, BTreeSet};

const NODE: Addr = Addr::Node(0);
const CLIENT: Addr = Addr::Client(1);
const S: usize = SECTOR_SIZE;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn cfg() -> Config {
    Config {
        initial_timeout: ms(100),
        max_timeout: ms(1000),
        max_attempts: 50,
    }
}
fn put(k: &str, v: &str) -> Request {
    Request::Put {
        key: k.into(),
        value: v.into(),
    }
}
fn get(k: &str) -> Request {
    Request::Get { key: k.into() }
}
fn del(k: &str) -> Request {
    Request::Delete { key: k.into() }
}
fn world(seed: u64, disk: DiskConfig) -> World {
    let mut w = World::cluster(
        1,
        WorldConfig {
            seed,
            disk,
            ..WorldConfig::default()
        },
    );
    w.add_client(1, cfg(), 0);
    w
}
fn view(w: &World, node: NodeId) -> Vec<Liveness> {
    let hb = w.heartbeat(node);
    hb.peers().map(|p| hb.liveness(p)).collect()
}

#[test]
fn a_restart_recovers_acknowledged_writes_from_the_disk_alone() {
    let mut w = world(0, DiskConfig::default());
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    assert_eq!(w.call(1, put("b", "2")), Response::PutOk);
    assert_eq!(w.call(1, del("a")), Response::DeleteOk { existed: true });
    assert_eq!(w.call(1, put("c", "3")), Response::PutOk);
    assert_eq!(w.disk(0).medium().len(), 4 * S);

    w.crash(0);
    assert!(!w.is_up(0));
    let rid = w.submit(1, get("c"));
    w.run_for(ms(50));
    assert_eq!(w.stats.dropped_at_down_hosts, 1, "nobody is listening");
    w.restart(0);
    w.run_until_idle();
    assert_eq!(
        w.take_completed(1, rid),
        Some(Response::Value(Some(b"3".to_vec())))
    );
    let node = w.node(0);
    assert_eq!(node.stats().recovered_entries, 4);
    assert_eq!(node.recovery().unwrap().tail, Tail::Clean);
    assert_eq!(node.get(b"a"), None);
    assert_eq!(node.get(b"b"), Some(&b"2"[..]));
    assert_eq!((w.stats.crashes, w.stats.restarts), (1, 1));
    assert_eq!(w.hosts[0].incarnation(), 2);
}

#[test]
fn a_crash_mid_append_tears_the_tail_and_the_retry_lands_once() {
    let mut seen = BTreeMap::new();
    for seed in 0..12 {
        let mut w = world(
            seed,
            DiskConfig {
                torn_write: Chance::ALWAYS,
                ..DiskConfig::default()
            },
        );
        assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
        // A two-sector record, in flight when the process dies.
        let rid = w.submit(1, put("b", &"x".repeat(600)));
        w.run_for(ms(2));
        assert_eq!(w.disk(0).pending(), 1);
        w.crash(0);
        let landed = w.disk(0).medium().len() / S - 1;
        assert!(landed < 2, "a torn write never lands whole");
        w.restart(0);
        w.run_until_idle();
        assert_eq!(
            w.take_completed(1, rid),
            Some(Response::PutOk),
            "the client retried"
        );
        let node = w.node(0);
        let tail = node.recovery().unwrap().tail;
        *seen.entry(landed).or_insert(0) += 1;
        match landed {
            0 => assert_eq!(tail, Tail::Clean),
            _ => assert_eq!(tail, Tail::Torn { at: S }),
        }
        assert_eq!(
            node.stats().recovered_entries,
            1,
            "the torn record is not an entry"
        );
        assert_eq!(node.log_seq(), 2, "the retry became record 2, once");
        assert_eq!(node.get(b"b").map(<[u8]>::len), Some(600));
        assert_eq!(w.disk(0).stats().torn, 1);
    }
    assert!(
        seen.len() == 2,
        "both zero and one sector landed across seeds: {seen:?}"
    );
}

#[test]
fn a_retry_across_a_restart_is_not_applied_twice() {
    // Before Module 6 put the request identity in the log, this test
    // answered `existed: false` and wrote a third record.
    let mut w = world(0, DiskConfig::default());
    assert_eq!(w.call(1, put("k", "v")), Response::PutOk);
    w.net.drop_next(NODE, CLIENT);
    let rid = w.submit(1, del("k"));
    w.run_for(ms(10));
    assert_eq!(w.node(0).log_seq(), 2, "the delete is durable");
    assert!(w.take_completed(1, rid).is_none(), "and the reply is gone");
    w.crash(0);
    w.restart(0);
    w.run_until_idle();
    assert_eq!(
        w.take_completed(1, rid),
        Some(Response::DeleteOk { existed: true }),
        "the session came back from the log"
    );
    let node = w.node(0);
    assert_eq!(node.log_seq(), 2, "no third record");
    assert_eq!(node.stats().duplicates, 1);
    assert_eq!(
        node.stats().applied,
        0,
        "nothing new was applied after the restart"
    );
    assert_eq!(node.get(b"k"), None);
}

/// Three acknowledged writes on one disk, as bytes.
fn three_records() -> Vec<u8> {
    let mut w = world(0, DiskConfig::default());
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    assert_eq!(w.call(1, put("b", &"x".repeat(600))), Response::PutOk);
    assert_eq!(w.call(1, put("c", "3")), Response::PutOk);
    let medium = w.disk(0).medium().to_vec();
    assert_eq!(medium.len(), 4 * S, "one, two and one sectors");
    medium
}

#[test]
fn every_rotten_sector_is_caught_and_the_last_record_is_the_ambiguous_one() {
    for sector in 0..4 {
        for seed in 0..5 {
            let mut disk = Disk::with_medium(DiskConfig::default(), three_records());
            disk.corrupt_sector(sector, &mut Prng::from_seed(seed));
            let mut w = World::new(disk);
            // A new client id: client 1's session is in the log.
            w.add_client(9, cfg(), 0);
            w.run_until_idle();
            let record = match sector {
                0 => 0,
                1 | 2 => 1,
                _ => 2,
            };
            if record < 2 {
                // A rotten record with records after it: a hole. The node halts.
                assert!(!w.is_up(0), "sector {sector} seed {seed}");
                let damage = w.hosts[0].last_halt.expect("halted over damage");
                assert_eq!(damage.at, record * S);
                assert_eq!(damage.seq, record as u64 + 1);
                assert_eq!(w.stats.halts, 1);
                let rid = w.submit(9, get("a"));
                w.run_for(ms(10));
                assert!(
                    w.take_completed(9, rid).is_none(),
                    "a halted node answers nothing"
                );
                assert_eq!(w.stats.dropped_at_down_hosts, 1);
            } else {
                // A rotten last record looks exactly like a torn one. The
                // node starts, with the acknowledged write to `c` gone and
                // nothing to say so. A single disk cannot tell the two
                // apart. That is why Module 10 replicates.
                assert!(w.is_up(0), "sector {sector} seed {seed}");
                let node = w.node(0);
                assert_eq!(node.stats().recovered_entries, 2);
                assert_eq!(node.recovery().unwrap().tail, Tail::Torn { at: 3 * S });
                assert_eq!(node.get(b"c"), None, "acknowledged, and gone");
                assert_eq!(w.call(9, get("b")), Response::Value(Some(b"x".repeat(600))));
            }
            // Whatever happened, no corrupted sector sits inside the part
            // of the log that recovery accepted.
            if w.is_up(0) {
                let valid_sectors = w.node(0).recovery().unwrap().valid_len as usize / S;
                assert!(w.disk(0).faults().all(|s| s >= valid_sectors));
            }
        }
    }
}

#[test]
fn a_lost_write_in_the_middle_halts_the_node() {
    let mut w = world(0, DiskConfig::default());
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    w.hosts[0].disk.lose_next_write();
    assert_eq!(w.call(1, put("b", "2")), Response::PutOk, "the fsync lied");
    assert_eq!(w.call(1, put("c", "3")), Response::PutOk);
    assert_eq!(w.disk(0).stats().lost, 1);
    assert!(
        w.disk(0).medium()[S..2 * S].iter().all(|&b| b == 0),
        "record 2 never landed"
    );
    w.crash(0);
    w.restart(0);
    w.run_until_idle();
    assert!(!w.is_up(0), "the node halted");
    let damage = w.hosts[0].last_halt.unwrap();
    assert_eq!((damage.at, damage.seq), (S, 2));
    assert_eq!((damage.next_valid_at, damage.next_valid_seq), (2 * S, 3));
    let halted: Vec<String> = w
        .trace()
        .lines()
        .iter()
        .filter(|l| l.contains("halted"))
        .cloned()
        .collect();
    assert_eq!(halted.len(), 1);
    assert!(
        halted[0].ends_with("record 2 at offset 512 is unreadable, record 3 is at offset 1024"),
        "{}",
        halted[0]
    );
    // A restart reads the same disk and halts again.
    w.restart(0);
    w.run_until_idle();
    assert!(!w.is_up(0));
    assert_eq!(w.stats.halts, 2);
}

#[test]
fn a_lost_write_at_the_tail_is_invisible() {
    let mut w = world(0, DiskConfig::default());
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    w.hosts[0].disk.lose_next_write();
    assert_eq!(w.call(1, put("b", "2")), Response::PutOk);
    w.crash(0);
    w.restart(0);
    w.run_until_idle();
    assert!(w.is_up(0));
    let node = w.node(0);
    assert_eq!(
        node.recovery().unwrap().tail,
        Tail::Clean,
        "nothing says a write was lost"
    );
    assert_eq!(node.stats().recovered_entries, 1);
    assert_eq!(node.get(b"b"), None, "acknowledged, and gone");
}

#[test]
fn a_misdirected_write_is_caught() {
    for seed in 0..10 {
        let mut w = world(seed, DiskConfig::default());
        assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
        w.hosts[0].disk.set_config(DiskConfig {
            misdirect_write: Chance::ALWAYS,
            ..DiskConfig::default()
        });
        assert_eq!(w.call(1, put("b", "2")), Response::PutOk);
        w.hosts[0].disk.set_config(DiskConfig::default());
        assert_eq!(w.call(1, put("c", "3")), Response::PutOk);
        assert_eq!(w.disk(0).stats().misdirected, 1);
        w.crash(0);
        w.restart(0);
        w.run_until_idle();
        assert!(!w.is_up(0), "seed {seed}: a record in the wrong place is a hole");
        let damage = w.hosts[0].last_halt.unwrap();
        assert!(damage.seq <= 2, "seed {seed}: {damage}");
    }
}

#[test]
fn a_read_fault_halts_the_node_and_a_clean_read_brings_it_back() {
    let mut halts = 0;
    for seed in 0..20 {
        let mut disk = Disk::with_medium(
            DiskConfig {
                corrupt_read: Chance::percent(40),
                ..DiskConfig::default()
            },
            three_records(),
        );
        let faulty_first_read = {
            // Peek at what the first read will do, with the same seed.
            let mut probe = disk.clone();
            let mut p = Prng::from_seed(seed);
            probe.submit_read(kv_core::Instant::ZERO, 0, &mut p);
            match probe.complete_next(&mut p) {
                sim::Completion::Read { faults, .. } => faults,
                _ => unreachable!(),
            }
        };
        let mut w = World::with_disks(
            vec![std::mem::take(&mut disk)],
            WorldConfig {
                seed,
                ..WorldConfig::default()
            },
        );
        w.add_client(9, cfg(), 0);
        w.run_until_idle();
        let faults = &w.hosts[0].last_read_faults;
        assert_eq!(*faults, faulty_first_read);
        // Records 1, 2 and 3 live in sectors {0}, {1, 2} and {3}. Recovery
        // halts exactly when a record read back wrong and a later record
        // read back right.
        let sectors_of = |record: usize| match record {
            0 => vec![0],
            1 => vec![1, 2],
            _ => vec![3],
        };
        let clean = |record: usize| sectors_of(record).iter().all(|s| !faults.contains(s));
        let first_bad = (0..3).find(|&r| !clean(r));
        let expect_halt = first_bad.is_some_and(|bad| (bad + 1..3).any(clean));
        assert_eq!(!w.is_up(0), expect_halt, "seed {seed} faults {faults:?}");
        if !w.is_up(0) {
            halts += 1;
            // The medium is fine. A reliable read recovers everything.
            w.hosts[0].disk.set_config(DiskConfig::default());
            w.restart(0);
            w.run_until_idle();
            assert!(w.is_up(0));
            assert_eq!(w.node(0).stats().recovered_entries, 3);
        }
        // Whatever happened, nothing corrupted was accepted.
        if w.is_up(0) {
            let valid_sectors = w.node(0).recovery().unwrap().valid_len as usize / S;
            assert!(w.hosts[0].last_read_faults.iter().all(|&s| s >= valid_sectors));
        }
    }
    assert!(halts > 0, "40% read faults never halted a node");
}

#[test]
fn a_crash_looks_like_a_partition_from_the_outside() {
    let mut w = World::cluster(3, WorldConfig::default());
    w.run_for(Duration::from_secs(1));
    w.crash(0);
    w.run_for(ms(500));
    assert_eq!(view(&w, 1), vec![Down, Up]);
    assert_eq!(view(&w, 2), vec![Down, Up]);
    assert!(
        w.stats.dropped_at_down_hosts >= 8,
        "beats to a dead host go nowhere"
    );
    w.restart(0);
    w.run_for(ms(200));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
    // The new process counts beats from one again, so its first beats
    // look like stragglers to its peers. Evidence that a restart happened,
    // seen from inside the protocol.
    assert!(w.heartbeat(1).stats().reordered > 0);
}

#[test]
fn the_quiet_period_brings_every_host_back() {
    let mut w = World::cluster(
        3,
        WorldConfig {
            seed: 5,
            crashes: Some(CrashSchedule {
                gap_max: Duration::from_secs(1),
                down_max: ms(400),
            }),
            ..WorldConfig::default()
        },
    );
    w.run_for(Duration::from_secs(5));
    assert!(w.stats.crashes >= 3, "{:?}", w.stats);
    assert_eq!(
        w.stats.crashes - w.stats.restarts,
        u64::from(!w.hosts.iter().all(|h| h.is_up()))
    );
    w.stop_crashes();
    assert!(w.hosts.iter().all(|h| h.is_up()));
    w.run_for(ms(350 + 100 + 1));
    assert_eq!(w.stats.crashes, w.stats.restarts);
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
    assert!(w.net.in_flight() <= 6);
}

/// What one client knows about its node: every write it issued, in
/// order, whether its fate is certain, and the state that implies.
#[derive(Default)]
struct Oracle {
    /// `(request id, entry, certain)`. A write is certain once the node
    /// acknowledged it and no restart has lost it. A write the client gave
    /// up on, or one a restart lost, may or may not be in the log. A lost
    /// write can even come back: a tail record that read back wrong once
    /// reads back right the next time, and the node has no memory of how
    /// long its log should be.
    issued: Vec<(u64, Entry, bool)>,
    state: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Keys whose value depends on a write of uncertain fate.
    uncertain: BTreeSet<Vec<u8>>,
    /// Acknowledged writes a restart lost, which the fault model allows
    /// and the lesson explains.
    lost: u64,
    /// The incarnation whose recovery was last checked.
    checked: u64,
}

fn key_of(entry: &Entry) -> &[u8] {
    match entry {
        Entry::Put { key, .. } | Entry::Delete { key } => key,
    }
}

impl Oracle {
    fn apply(state: &mut BTreeMap<Vec<u8>, Vec<u8>>, entry: &Entry) {
        match entry {
            Entry::Put { key, value } => {
                state.insert(key.clone(), value.clone());
            }
            Entry::Delete { key } => {
                state.remove(key);
            }
        }
    }

    fn ack(&mut self, rid: u64, entry: Entry) {
        Oracle::apply(&mut self.state, &entry);
        self.uncertain.remove(key_of(&entry));
        self.issued.push((rid, entry, true));
    }

    fn gave_up(&mut self, rid: u64, entry: Entry) {
        self.uncertain.insert(key_of(&entry).to_vec());
        self.issued.push((rid, entry, false));
    }

    /// A new process has recovered. What it found must be what the client
    /// issued, in order, with uncertain writes present or absent, and
    /// certain writes missing only on a disk that can lose them. A
    /// certain write that is missing becomes uncertain.
    fn check_restart(&mut self, w: &World, node: NodeId, disk: &DiskConfig) {
        let host = &w.hosts[usize::from(node)];
        if !host.is_up() || host.incarnation() == self.checked {
            return;
        }
        let Some(recovery) = host.node().recovery() else {
            return; // still reading the disk
        };
        self.checked = host.incarnation();
        let mut found = host.node().recovered_records().iter().peekable();
        let mut lost_from = None;
        self.state.clear();
        self.uncertain.clear();
        for (rid, entry, certain) in &mut self.issued {
            match found.peek() {
                Some(r) if r.request_id == *rid => {
                    assert_eq!(&r.entry, entry, "node {node} record {rid} differs");
                    found.next();
                    *certain = true;
                    Oracle::apply(&mut self.state, entry);
                    self.uncertain.remove(key_of(entry));
                }
                _ => {
                    if *certain {
                        *certain = false;
                        lost_from.get_or_insert(*rid);
                        self.lost += 1;
                    }
                    self.uncertain.insert(key_of(entry).to_vec());
                }
            }
        }
        if let Some(rid) = lost_from {
            let can_lose = !disk.lost_write.is_never()
                || !disk.corrupt_write.is_never()
                || !disk.corrupt_read.is_never();
            assert!(
                can_lose,
                "node {node} lost acknowledged write {rid} on a disk that cannot lose one"
            );
        }
        // At most the write in flight right now may be in the log already.
        assert!(
            found.next().is_none_or(|_| found.peek().is_none()),
            "node {node} has records nobody issued"
        );
        let valid_sectors = recovery.valid_len as usize / S;
        assert!(
            host.last_read_faults.iter().all(|&s| s >= valid_sectors),
            "node {node} accepted a sector that read back wrong"
        );
        assert!(
            host.disk.faults().all(|s| s >= valid_sectors),
            "node {node} accepted a sector that rotted"
        );
    }

    /// The client is idle: every key with a known value must match.
    fn check_state(&self, w: &World, node: NodeId, keys: &[Vec<u8>]) {
        if !w.is_up(node) {
            return;
        }
        let n = w.node(node);
        for key in keys {
            if self.uncertain.contains(key) {
                continue;
            }
            assert_eq!(
                n.get(key),
                self.state.get(key).map(Vec::as_slice),
                "node {node} key {key:?}"
            );
        }
    }
}

/// Three hosts, one client each, under random network faults, random disk
/// faults, random partitions and random crashes, all from the seed. Then
/// the faults stop and every host must come back. The trace must be a
/// pure function of the seed.
fn chaos(prng: &mut Prng, trace: &mut determinism::Trace) {
    let disk = DiskConfig::random(prng);
    let config = WorldConfig {
        seed: prng.next_u64(),
        net: NetConfig::random(prng),
        disk: disk.clone(),
        partitions: Some(PartitionSchedule {
            gap_max: Duration::from_secs(2),
            hold_max: Duration::from_secs(1),
        }),
        crashes: Some(CrashSchedule {
            gap_max: Duration::from_secs(1),
            down_max: ms(500),
        }),
        ..WorldConfig::default()
    };
    let mut w = World::cluster(3, config);
    let mut oracles: Vec<Oracle> = Vec::new();
    for node in 0..3 {
        w.add_client(u64::from(node) + 1, cfg(), node);
        oracles.push(Oracle::default());
    }
    let keys: Vec<Vec<u8>> = (0..4).map(|i| format!("k{i}").into_bytes()).collect();
    let mut skipped = 0;
    let mut completed = 0;
    for i in 0..120u64 {
        let node = prng.below(3) as NodeId;
        let id = u64::from(node) + 1;
        if !w.is_up(node) && w.hosts[usize::from(node)].last_halt.is_some() {
            // Halted over damage. Nothing will answer; do not wait on it.
            skipped += 1;
            continue;
        }
        let key = prng.pick(&keys).clone();
        let (request, entry) = match prng.below(3) {
            0 => (Request::Get { key }, None),
            1 => {
                let value = format!("v{i}").into_bytes();
                (
                    Request::Put {
                        key: key.clone(),
                        value: value.clone(),
                    },
                    Some(Entry::Put { key, value }),
                )
            }
            _ => (Request::Delete { key: key.clone() }, Some(Entry::Delete { key })),
        };
        let rid = w.submit(id, request);
        loop {
            // Stop at every disk read completion, so that a restarted
            // node is checked against the oracle before it serves.
            let reads = w.stats.reads_completed;
            assert!(w.run_while(|w| !w.clients[&id].is_idle() && w.stats.reads_completed == reads));
            for (n, oracle) in oracles.iter_mut().enumerate() {
                oracle.check_restart(&w, n as NodeId, &disk);
            }
            if w.clients[&id].is_idle() {
                break;
            }
        }
        let oracle = &mut oracles[usize::from(node)];
        if w.take_completed(id, rid).is_some() {
            completed += 1;
            if let Some(entry) = entry {
                oracle.ack(rid, entry);
            }
            oracle.check_state(&w, node, &keys);
        } else {
            assert!(w.gave_up.contains(&(id, rid)));
            if let Some(entry) = entry {
                oracle.gave_up(rid, entry);
            }
        }
    }

    // The quiet period.
    w.stop_partitions();
    w.stop_crashes();
    w.net.set_config(NetConfig::reliable(ms(1)));
    for host in &mut w.hosts {
        host.disk.set_config(DiskConfig::default());
    }
    w.run_until_idle();
    w.run_for(ms(350 + 100 + 1));
    for (n, oracle) in oracles.iter_mut().enumerate() {
        oracle.check_restart(&w, n as NodeId, &disk);
        oracle.check_state(&w, n as NodeId, &keys);
    }
    let up: Vec<NodeId> = w.hosts.iter().filter(|h| h.is_up()).map(|h| h.id).collect();
    for &a in &up {
        let hb = w.heartbeat(a);
        for &b in &up {
            if a != b {
                assert_eq!(hb.liveness(b), Up, "node {a} does not see node {b}");
            }
        }
    }
    for line in w.trace().lines() {
        trace.record(format_args!("{line}"));
    }
    let lost: u64 = oracles.iter().map(|o| o.lost).sum();
    determinism::trace!(
        trace,
        "chaos {:?} completed={completed} gave_up={} skipped={skipped} lost={lost} halted={}",
        w.stats,
        w.gave_up.len(),
        3 - up.len()
    );
}

#[test]
fn storage_chaos_is_a_pure_function_of_the_seed() {
    for seed in 0..20 {
        check_determinism(seed, 3, chaos).unwrap_or_else(|d| panic!("{d}"));
    }
}

#[test]
fn known_storage_fingerprint() {
    // Pins seed 1 of the storage chaos run. Any change to a trace line in
    // the node, client, heartbeat, scheduler, network model, disk model or
    // world changes it. Module 7 changed it on purpose: the configuration
    // of the run heads every trace.
    let hash = check_determinism(1, 2, chaos).unwrap();
    assert_eq!(
        hash, 0xea7a_6198_ba24_4e5d,
        "seed 1 storage chaos hash changed: {hash:#018x}"
    );
}
