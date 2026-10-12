//! Module 10 acceptance tests, part one: consensus by hand.
//!
//! Three hosts are one store, and two of them are enough. A write is
//! answered once a majority of disks have it. The primary of view `v` is
//! node `v mod 3`. A backup that suspects the primary starts a view
//! change, a majority agrees to it, the new primary takes the newest of
//! their logs and serves once a majority holds it. Every promise is on
//! disk before it is spoken. A read waits for a majority's beats. A
//! damaged log is repaired record by record along the chain.

use determinism::Prng;
use kv_core::client::Config;
use kv_core::replica::Status;
use kv_core::{log, superblock};
use kv_core::{Duration, NodeId, Request, Response};
use kv_env::fake::{Addr, World, WorldConfig};
use sim::NetConfig;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn cfg() -> Config {
    Config {
        initial_timeout: ms(100),
        max_timeout: ms(400),
        max_attempts: 8,
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
fn value(v: &str) -> Response {
    Response::Value(Some(v.into()))
}
/// Three hosts, one client each, reliable everything.
fn cluster(seed: u64) -> World {
    let mut w = World::cluster(
        3,
        WorldConfig {
            seed,
            ..WorldConfig::default()
        },
    );
    for node in 0..3 {
        w.add_client(u64::from(node) + 1, cfg(), node);
    }
    w
}
fn settle(w: &mut World) {
    assert!(
        w.run_until_converged(Duration::from_secs(5)),
        "the cluster did not settle"
    );
}
/// Records on `node`'s disk, decoded.
fn on_disk(w: &World, node: NodeId) -> usize {
    log::recover(w.log_on_disk(node)).records.len()
}
/// The committed prefixes of every pair of logs agree. The invariant
/// that replaces Module 9's "logs are prefixes of one another": an
/// uncommitted tail may differ, a committed record never does.
fn check_committed(w: &World) -> Result<(), (NodeId, NodeId)> {
    let logs: Vec<(NodeId, u64, &[u8])> = w
        .hosts
        .iter()
        .filter(|h| h.is_up())
        .map(|h| (h.id, h.replica().commit(), h.replica().log_bytes()))
        .collect();
    for (i, &(a, ca, la)) in logs.iter().enumerate() {
        for &(b, cb, lb) in &logs[i + 1..] {
            let n = usize::try_from(ca.min(cb)).unwrap() * log::SECTOR_SIZE;
            let n = n.min(la.len()).min(lb.len());
            if la[..n] != lb[..n] {
                return Err((a, b));
            }
        }
    }
    Ok(())
}

#[test]
fn three_hosts_are_one_store_and_two_disks_are_enough() {
    let mut w = cluster(1);
    settle(&mut w);
    assert_eq!(w.primary(), Some(0), "the primary of view 0 is node 0");
    for n in 0..3 {
        assert_eq!(w.replica(n).view(), 0);
        assert_eq!(w.replica(n).promised(), 0);
    }
    assert_eq!(w.replica(1).status(), Status::Backup);
    assert_eq!(
        w.call(2, put("a", "1")),
        Response::PutOk,
        "written through node 1"
    );
    assert_eq!(w.call(3, get("a")), value("1"), "read through node 2");
    assert_eq!(w.replica(0).stats().commits, 1);
    assert_eq!(w.replica(0).stats().reads, 1);
    // Two of three. A write is answered once one backup has it.
    w.crash(2);
    let rid = w.submit(1, put("b", "2"));
    let answered = |w: &World| w.completed.iter().any(|(c, r, _)| (*c, *r) == (1, rid));
    assert!(w.run_while(|w| !answered(w)));
    assert_eq!(on_disk(&w, 0), 2);
    assert_eq!(on_disk(&w, 1), 2);
    assert_eq!(w.replica(0).commit(), 2);
    // The third comes back and is pushed what it missed.
    w.restart(2);
    settle(&mut w);
    assert_eq!(on_disk(&w, 2), 2);
    assert_eq!(w.replica(2).stats().prepares_accepted, 1);
    assert_eq!(w.check_logs(), Ok(()));
}

#[test]
fn the_primary_crashes_and_a_majority_elects_the_next_view() {
    let mut w = cluster(3);
    settle(&mut w);
    assert_eq!(w.call(2, put("a", "1")), Response::PutOk);
    w.crash(0);
    // The backups suspect the primary, agree on view 1, and node 1
    // serves without waiting for node 0.
    let rid = w.submit(3, get("a"));
    assert!(w.run_while(|w| !w.clients[&3].is_idle()));
    assert_eq!(w.take_completed(3, rid), Some(value("1")));
    assert_eq!(w.primary(), Some(1));
    assert_eq!(w.replica(1).view(), 1);
    assert_eq!(w.replica(2).status(), Status::Backup);
    assert_eq!(
        w.replica(2).promised(),
        1,
        "the vote was written before it was cast"
    );
    assert_eq!(w.call(2, put("b", "2")), Response::PutOk, "two disks");
    // Node 0 comes back, finds view 1 in its beats, and follows.
    w.restart(0);
    settle(&mut w);
    assert_eq!(w.replica(0).status(), Status::Backup);
    assert_eq!(w.replica(0).view(), 1);
    assert_eq!(w.replica(1).stats().promotions, 1);
    assert!(w.replica(1).stats().view_changes_started + w.replica(2).stats().view_changes_started >= 1);
    for n in 0..3 {
        assert_eq!(on_disk(&w, n), 2, "node {n}");
    }
    assert_eq!(w.check_logs(), Ok(()));
}

#[test]
fn an_isolated_primary_answers_nothing_and_its_write_is_redone_in_the_next_view() {
    let mut w = cluster(4);
    w.add_client(4, cfg(), 0);
    settle(&mut w);
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    let nodes = w.node_addrs();
    w.net.isolate(Addr::Node(0), &nodes[1..]);
    // The client of the isolated primary writes. The primary appends the
    // record and sends prepares that never arrive. No reply.
    let rid = w.submit(1, put("a", "2"));
    w.run_for(ms(600));
    assert!(w.take_completed(1, rid).is_none());
    assert_eq!(w.replica(0).len(), 2, "prepared on the old primary");
    assert_eq!(w.replica(0).uncommitted(), 1);
    assert_eq!(w.replica(1).view(), 1, "the backups moved on");
    // A read at the old primary is not answered either: no peer's beat
    // in its view arrives to prove it is still the primary.
    let read = w.submit(4, get("a"));
    w.run_for(ms(300));
    assert!(w.take_completed(4, read).is_none(), "no stale read");
    assert!(w.replica(0).pending_reads() >= 1, "held, and the retry held too");
    assert_eq!(w.replica(1).status(), Status::Primary, "two were enough");
    // Heal. The old primary hears view 1, steps down and drops its
    // uncommitted record. The client's retry is forwarded to node 1,
    // which writes it anew.
    w.net.heal_all();
    settle(&mut w);
    assert_eq!(w.primary(), Some(1));
    assert_eq!(w.replica(0).stats().step_downs, 1);
    assert!(
        w.replica(0).stats().truncations >= 1,
        "the uncommitted record went"
    );
    assert_eq!(w.replica(0).uncommitted(), 0);
    assert!(w.run_while(|w| !w.clients[&1].is_idle()));
    assert_eq!(w.take_completed(1, rid), Some(Response::PutOk));
    assert!(w.run_while(|w| !w.clients[&4].is_idle()));
    assert_eq!(
        w.take_completed(4, read),
        Some(value("2")),
        "answered after the write"
    );
    assert_eq!(w.check_logs(), Ok(()));
    for n in 0..3 {
        assert_eq!(on_disk(&w, n), 2, "node {n}");
    }
}

#[test]
fn a_read_waits_for_a_majority_of_beats() {
    let mut w = cluster(5);
    settle(&mut w);
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    // Just after a round of beats: the next one is a hundred milliseconds
    // away, and the read waits for it.
    let next_beat = w.heartbeat(0).next_deadline().unwrap();
    w.run_until(next_beat + ms(5));
    let rid = w.submit(1, get("a"));
    w.run_for(ms(10));
    assert_eq!(w.replica(0).pending_reads(), 1, "held for the beats");
    assert!(w.take_completed(1, rid).is_none());
    w.run_for(ms(110));
    assert_eq!(w.take_completed(1, rid), Some(value("1")));
    assert_eq!(w.replica(0).pending_reads(), 0);
    assert_eq!(w.replica(0).stats().reads, 1);
}

#[test]
fn the_promise_is_on_disk_before_the_vote_and_survives_a_restart() {
    let mut w = cluster(6);
    settle(&mut w);
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    w.crash(0);
    settle_without(&mut w, 0);
    // Every replica in view 1 wrote the superblock before it spoke.
    for n in 1..3 {
        let sb = superblock::read(w.disk(n).medium()).expect("a superblock");
        assert_eq!(sb.view, 1, "node {n}");
        assert_eq!(w.replica(n).promised(), 1);
        let lines = w.trace();
        let lines = lines.lines();
        let vote = lines
            .iter()
            .position(|l| l.contains(&format!("n{n} out Send")) && l.contains("start_view_change view=1"))
            .expect("a vote");
        // The superblock write before the vote, and its completion before
        // the vote too.
        let promise = lines[..vote]
            .iter()
            .rev()
            .find(|l| {
                l.contains(&format!("n{n} out Append")) && l.contains("offset=") && {
                    let off: u64 = l
                        .split("offset=")
                        .nth(1)
                        .unwrap()
                        .split(' ')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    off < superblock::LOG_OFFSET as u64
                }
            })
            .expect("a superblock write");
        let op = promise.split("op=").nth(1).unwrap().split(' ').next().unwrap();
        assert!(
            lines[..vote]
                .iter()
                .any(|l| l.contains(&format!("n{n} in  AppendComplete op={op}"))),
            "node {n} spoke before its promise was durable"
        );
    }
    // A restarted replica finds its promise and does not go back.
    w.crash(2);
    w.restart(2);
    w.run_for(ms(1));
    assert!(w.replica(2).view() >= 1);
    w.restart(0);
    settle(&mut w);
    assert_eq!(w.replica(2).promised(), 1);
    assert_eq!(w.check_logs(), Ok(()));
}

/// Run until every host but `down` is a backup or the primary of one
/// view with equal, durable logs.
fn settle_without(w: &mut World, down: NodeId) {
    let ok = w.run_while(|w| {
        let up: Vec<&kv_env::fake::Host> = w.hosts.iter().filter(|h| h.id != down).collect();
        let primary = up.iter().find(|h| h.replica().is_primary());
        !primary.is_some_and(|p| {
            up.iter().all(|h| {
                matches!(h.replica().status(), Status::Backup | Status::Primary)
                    && h.replica().view() == p.replica().view()
                    && h.replica().log_bytes() == p.replica().log_bytes()
                    && h.replica().durable() == h.replica().len()
            })
        }) && w.now() < kv_core::Instant::ZERO + Duration::from_secs(5)
    });
    assert!(ok);
    assert!(
        w.now() < kv_core::Instant::ZERO + Duration::from_secs(5),
        "did not settle"
    );
}

#[test]
fn a_damaged_log_is_repaired_along_the_chain() {
    for victim in [1u8, 0u8] {
        let mut w = cluster(7 + u64::from(victim));
        settle(&mut w);
        for i in 0..4 {
            assert_eq!(w.call(2, put("k", &i.to_string())), Response::PutOk);
        }
        w.crash(victim);
        // Record 2 rots on the victim's disk, with records after it.
        w.hosts[usize::from(victim)].disk.corrupt_sector(
            superblock::LOG_OFFSET / log::SECTOR_SIZE + 1,
            &mut Prng::from_seed(9),
        );
        w.restart(victim);
        settle(&mut w);
        let r = w.replica(victim);
        let stats = r.stats();
        assert_eq!(stats.repairs, 1, "victim {victim}");
        assert!(matches!(r.recovery().unwrap().tail, log::Tail::Damaged(_)));
        assert_eq!(stats.salvaged, 2, "records 3 and 4 were kept");
        assert_eq!(stats.strays, 0, "and both chained to record 2");
        assert_eq!(stats.erasures, 0);
        assert_eq!(r.len(), 4);
        assert_eq!(r.durable(), 4);
        assert_eq!(on_disk(&w, victim), 4, "the disk is whole again");
        assert_eq!(w.check_logs(), Ok(()));
        let primary = w.primary().unwrap();
        assert_eq!(w.replica(victim).log_bytes(), w.replica(primary).log_bytes());
        assert_eq!(w.call(3, get("k")), value("3"));
    }
}

#[test]
fn committed_prefixes_agree_through_the_swarm() {
    for seed in 0..12 {
        let mut prng = Prng::from_seed(seed);
        let mut config = WorldConfig::swarm(&mut prng);
        config.net = NetConfig::random(&mut prng);
        let mut w = World::cluster(3, config);
        for node in 0..3 {
            w.add_client(u64::from(node) + 1, cfg(), node);
        }
        let keys = [b"a".to_vec(), b"b".to_vec()];
        for i in 0..80u64 {
            let id = 1 + prng.below(3);
            let key = prng.pick(&keys).clone();
            let request = if prng.chance(1, 2) {
                Request::Get { key }
            } else {
                Request::Put {
                    key,
                    value: format!("v{i}").into_bytes(),
                }
            };
            w.submit(id, request);
            // The invariant holds at every event, not only at the end.
            let mut steps = 0;
            assert!(w.run_while(|w| {
                steps += 1;
                if steps % 25 == 0 {
                    check_committed(w).unwrap_or_else(|(a, b)| {
                        panic!("seed {seed}: committed logs of n{a} and n{b} differ")
                    });
                }
                !w.clients[&id].is_idle()
            }));
        }
        w.stop_faults();
        assert!(
            w.run_until_converged(Duration::from_secs(10)),
            "seed {seed} did not converge"
        );
        assert_eq!(w.check_logs(), Ok(()), "seed {seed}");
        for n in 0..3 {
            assert_eq!(
                w.replica(n).promised(),
                w.replica(n).view(),
                "seed {seed} node {n}"
            );
        }
    }
}
