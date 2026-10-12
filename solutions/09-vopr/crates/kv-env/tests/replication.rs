//! Module 9 acceptance tests, part one: the replicated store by hand.
//!
//! Three hosts are one store. A write is answered only when it is on
//! every disk. The primary of view `v` is node `v mod 3`, a backup whose
//! detector marks the primary `Down` moves to the next view, and the new
//! primary serves only after every replica reported, the longest log was
//! pulled and every disk has it. Logs are prefixes of one another at all
//! times, so no record is ever thrown away. A damaged log is repaired from
//! a peer.

use determinism::Prng;
use kv_core::client::Config;
use kv_core::log;
use kv_core::replica::Status;
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
    log::recover(w.disk(node).medium()).records.len()
}

#[test]
fn three_hosts_are_one_store_and_the_backups_forward() {
    let mut w = cluster(1);
    settle(&mut w);
    assert_eq!(w.primary(), Some(0), "the primary of view 0 is node 0");
    for n in 0..3 {
        assert_eq!(w.replica(n).view(), 0);
    }
    assert_eq!(w.replica(1).status(), Status::Backup);
    assert_eq!(
        w.call(2, put("a", "1")),
        Response::PutOk,
        "written through node 1"
    );
    assert_eq!(w.call(3, get("a")), value("1"), "read through node 2");
    assert_eq!(w.call(1, get("a")), value("1"), "read at the primary");
    assert_eq!(w.replica(1).stats().forwarded, 1);
    assert_eq!(w.replica(2).stats().forwarded, 1);
    assert_eq!(w.replica(0).stats().requests, 3);
    // The reply came from the primary, not from the node the client
    // talked to.
    let replies: Vec<String> = w
        .trace()
        .lines()
        .iter()
        .filter(|l| l.contains("net send n") && l.contains(" reply "))
        .cloned()
        .collect();
    assert_eq!(replies.len(), 3);
    assert!(
        replies.iter().all(|l| l.contains("net send n0->c")),
        "{replies:?}"
    );
}

#[test]
fn a_write_is_on_every_disk_before_it_is_answered() {
    let mut w = cluster(2);
    settle(&mut w);
    let rid = w.submit(1, put("a", "1"));
    // Stop at the reply, before the client's next step.
    let answered = |w: &World| w.completed.iter().any(|(c, r, _)| (*c, *r) == (1, rid));
    assert!(w.run_while(|w| !answered(w)));
    for n in 0..3 {
        assert_eq!(
            on_disk(&w, n),
            1,
            "node {n} has the record before the client hears"
        );
        assert_eq!(w.replica(n).durable(), 1);
    }
    assert_eq!(w.replica(0).stats().commits, 1);
    assert_eq!(w.replica(0).stats().prepares_sent, 2);
    assert_eq!(w.replica(1).stats().prepares_accepted, 1);
    assert_eq!(w.check_logs(), Ok(()));
}

#[test]
fn the_primary_crashes_and_the_next_view_serves_what_it_acknowledged() {
    let mut w = cluster(3);
    settle(&mut w);
    assert_eq!(w.call(2, put("a", "1")), Response::PutOk);
    w.crash(0);
    // The backups suspect the primary and move to view 1, whose primary
    // is node 1. It gathers: every replica must report in view 1, and
    // node 0 is down, so it waits.
    w.run_for(ms(500));
    assert_eq!(w.replica(1).view(), 1);
    assert_eq!(
        w.replica(1).status(),
        Status::Gathering,
        "nobody serves without node 0"
    );
    assert_eq!(w.replica(2).status(), Status::Backup);
    let rid = w.submit(3, get("a"));
    w.run_for(ms(300));
    assert!(
        w.take_completed(3, rid).is_none(),
        "the cluster blocks while a replica is down"
    );
    // Node 0 comes back, reads its disk, hears view 1 and reports. Node 1
    // becomes the primary and answers, with the write still there.
    w.restart(0);
    settle(&mut w);
    assert_eq!(w.primary(), Some(1));
    assert_eq!(w.replica(0).status(), Status::Backup);
    assert_eq!(w.replica(0).view(), 1);
    assert_eq!(w.replica(1).stats().promotions, 1);
    assert!(w.run_while(|w| !w.clients[&3].is_idle()));
    assert_eq!(
        w.take_completed(3, rid),
        Some(value("1")),
        "the retry was answered"
    );
    assert_eq!(w.call(3, get("a")), value("1"));
    assert_eq!(w.call(1, put("b", "2")), Response::PutOk, "node 0 forwards now");
    assert_eq!(w.replica(0).stats().forwarded, 1);
    for n in 0..3 {
        assert_eq!(on_disk(&w, n), 2, "node {n}");
    }
    assert_eq!(w.check_logs(), Ok(()));
}

#[test]
fn an_isolated_primary_prepares_but_cannot_commit_and_the_record_survives() {
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
    assert_eq!(w.replica(1).len(), 1);
    assert_eq!(w.replica(1).view(), 1, "the backups moved on");
    assert_eq!(w.replica(1).status(), Status::Gathering);
    // The old primary still answers reads: nothing newer has been
    // committed anywhere, so they are not stale. But it never commits.
    assert_eq!(w.call(4, get("a")), value("1"));
    // Heal. The old primary hears view 1 and steps down. The new primary
    // pulls the record it never saw, pushes it everywhere and serves it.
    // The client's retry is answered from the session the record made.
    w.net.heal_all();
    settle(&mut w);
    assert_eq!(w.primary(), Some(1));
    assert_eq!(w.replica(0).stats().step_downs, 1);
    assert!(w.replica(1).stats().pulls_sent >= 1, "{:?}", w.replica(1).stats());
    assert_eq!(w.replica(1).len(), 2);
    assert!(w.run_while(|w| !w.clients[&1].is_idle()));
    assert_eq!(
        w.take_completed(1, rid),
        Some(Response::PutOk),
        "answered from the session"
    );
    assert_eq!(
        w.call(2, get("a")),
        value("2"),
        "the uncommitted record was committed"
    );
    assert_eq!(w.check_logs(), Ok(()));
    for n in 0..3 {
        assert_eq!(on_disk(&w, n), 2, "node {n}");
    }
}

#[test]
fn a_damaged_log_is_repaired_from_the_peers() {
    for victim in [1u8, 0u8] {
        let mut w = cluster(5 + u64::from(victim));
        settle(&mut w);
        for i in 0..4 {
            assert_eq!(w.call(2, put("k", &i.to_string())), Response::PutOk);
        }
        w.crash(victim);
        // Record 2 rots on the victim's disk, with records after it.
        w.hosts[usize::from(victim)]
            .disk
            .corrupt_sector(1, &mut Prng::from_seed(9));
        w.restart(victim);
        settle(&mut w);
        let r = w.replica(victim);
        let stats = r.stats();
        assert_eq!(stats.repairs, 1, "victim {victim}");
        assert!(matches!(r.recovery().unwrap().tail, log::Tail::Damaged(_)));
        assert_eq!(stats.salvaged, 2, "records 3 and 4 were kept");
        assert_eq!(stats.fetched, 1, "record 2 came from a peer");
        assert_eq!(stats.erasures, 0);
        assert_eq!(r.len(), 4);
        assert_eq!(r.durable(), 4);
        assert_eq!(on_disk(&w, victim), 4, "the disk is whole again");
        assert_eq!(w.check_logs(), Ok(()));
        let primary = w.primary().unwrap();
        assert_eq!(w.replica(victim).log_bytes(), w.replica(primary).log_bytes());
        let repairing: Vec<String> = w
            .trace()
            .lines()
            .iter()
            .filter(|l| l.contains(&format!("n{victim} status view=")) && l.contains("repairing"))
            .cloned()
            .collect();
        assert_eq!(repairing.len(), 1, "victim {victim}: {repairing:?}");
        assert_eq!(w.call(3, get("k")), value("3"));
    }
}

#[test]
fn a_restarted_backup_catches_up_and_a_lost_prepare_is_pushed_again() {
    let mut w = cluster(8);
    settle(&mut w);
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    w.crash(2);
    // Two writes while node 2 is down: nothing commits.
    let r1 = w.submit(1, put("b", "2"));
    w.run_for(ms(300));
    assert!(w.take_completed(1, r1).is_none());
    assert_eq!(w.replica(0).len(), 2);
    assert_eq!(w.replica(0).uncommitted(), 1);
    w.restart(2);
    settle(&mut w);
    assert!(w.run_while(|w| !w.clients[&1].is_idle()));
    assert_eq!(w.take_completed(1, r1), Some(Response::PutOk));
    assert_eq!(w.replica(2).len(), 2);
    // The new process counts from its restart: record 1 was on its disk,
    // record 2 was pushed to it.
    assert_eq!(w.replica(2).stats().prepares_accepted, 1);

    // A prepare that the network loses is pushed again after the next
    // beat says the backup is behind.
    w.net.drop_next(Addr::Node(0), Addr::Node(1));
    assert_eq!(w.call(1, put("c", "3")), Response::PutOk);
    assert!(
        w.replica(0).stats().prepares_sent >= 5,
        "{:?}",
        w.replica(0).stats()
    );
    assert!(w.replica(1).stats().prepares_ignored == 0 || w.replica(1).len() == 3);
    assert_eq!(w.check_logs(), Ok(()));
}

#[test]
fn logs_stay_prefixes_of_one_another_through_the_swarm() {
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
                    w.check_logs()
                        .unwrap_or_else(|(a, b)| panic!("seed {seed}: logs of n{a} and n{b} diverged"));
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
    }
}
