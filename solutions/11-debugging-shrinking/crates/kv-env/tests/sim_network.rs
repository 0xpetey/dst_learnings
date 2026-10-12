//! Module 5 acceptance tests: three hosts pass heartbeats through the
//! simulated network while the seed, or the test, breaks it.
//!
//! Three things must hold whatever the faults:
//!
//! 1. The network never delivers a message that nobody sent.
//! 2. Partitioned nodes never exchange messages.
//! 3. With faults disabled, delivery is in order and lossless.

use determinism::{check_determinism, Prng};
use kv_core::client::Config;
use kv_core::heartbeat::Liveness::{self, Down, Up};
use kv_core::{Duration, Instant, NodeId, Request, Response};
use kv_env::fake::{Addr, Event, PartitionSchedule, World, WorldConfig};
use sim::{Chance, Disk, NetConfig};

const N0: Addr = Addr::Node(0);
const N1: Addr = Addr::Node(1);
const N2: Addr = Addr::Node(2);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn at(millis: u64) -> Instant {
    Instant::ZERO + ms(millis)
}
fn cluster(net: NetConfig) -> World {
    World::cluster(
        3,
        WorldConfig {
            net,
            ..WorldConfig::default()
        },
    )
}

/// What `node` believes about every other node, in id order.
fn view(w: &World, node: NodeId) -> Vec<Liveness> {
    let hb = w.heartbeat(node);
    hb.peers().map(|p| hb.liveness(p)).collect()
}

/// The virtual timestamp at the start of a trace line.
fn stamp(line: &str) -> u64 {
    let digits = line.strip_prefix("t=").expect("trace lines start with t=");
    digits[..digits.find("ns").unwrap()].parse().unwrap()
}

/// Deliveries between `a` and `b`, either way, strictly after `after`.
fn deliveries_between(w: &World, a: Addr, b: Addr, after: Instant) -> usize {
    let ab = format!("net deliver {a}->{b} ");
    let ba = format!("net deliver {b}->{a} ");
    w.trace()
        .lines()
        .iter()
        .filter(|l| stamp(l) > after.nanos() && (l.contains(&ab) || l.contains(&ba)))
        .count()
}

#[test]
fn a_reliable_network_is_in_order_and_lossless() {
    let mut w = cluster(NetConfig::reliable(ms(1)));
    w.run_for(Duration::from_secs(10));
    let s = w.net.stats();
    assert_eq!((s.lost, s.partitioned, s.duplicated, s.reordered), (0, 0, 0, 0));
    // Beats at 0, 100, ..., 10000 ms: 101 rounds to two peers per host,
    // and the fourteen messages that started view 0: six
    // `StartViewChange`, two `DoViewChange`, two `StartView`, two
    // `PrepareOk` for the empty log and two beats-worth of nothing.
    assert_eq!(s.sent, 3 * 2 * 101 + 14);
    // The last round is still on the wire at the bound.
    assert_eq!(s.sent - s.delivered, w.net.in_flight() as u64);
    assert_eq!(w.net.in_flight(), 6);
    for host in &w.hosts {
        let hs = host.heartbeat().stats();
        assert_eq!(hs.sent, 202);
        assert_eq!(hs.received, 200);
        assert_eq!((hs.duplicates, hs.reordered), (0, 0), "in order, every beat once");
        assert_eq!((hs.ups, hs.downs), (2, 0), "no false suspicion");
        assert_eq!(view(&w, host.id), vec![Up, Up]);
    }
    assert!(w.is_idle(), "heartbeats do not count as work");
}

#[test]
fn clients_reach_the_node_they_were_given() {
    let mut w = cluster(NetConfig::reliable(ms(1)));
    w.add_client(1, Config::default(), 0);
    w.add_client(2, Config::default(), 2);
    let put = Request::Put {
        key: b"k".into(),
        value: b"v".into(),
    };
    let get = Request::Get { key: b"k".into() };
    assert_eq!(w.call(1, put), Response::PutOk);
    assert_eq!(w.call(1, get.clone()), Response::Value(Some(b"v".to_vec())));
    // One store since Module 9: node 2 forwards to the primary, node 0.
    assert_eq!(w.call(2, get), Response::Value(Some(b"v".to_vec())));
    assert_eq!(w.primary(), Some(0));
    assert_eq!(w.node(0).len(), 1);
    assert_eq!(w.replica(2).stats().forwarded, 1);
    assert_eq!(w.replica(0).stats().requests, 3);
    assert_eq!(w.stats.requests_delivered, 3);
    assert_eq!(w.stats.replies_delivered, 3);
}

#[test]
fn a_symmetric_partition_is_detected_and_heals() {
    let mut w = cluster(NetConfig::reliable(ms(1)));
    w.run_for(Duration::from_secs(1));
    let cut_at = w.now();
    w.net.split(&[N0], &[N1, N2]);
    // The beats sent at 1000 ms are in flight when the cable is cut. They
    // are lost. The last beat n0 heard arrived at 901 ms, so n0 suspects
    // both peers at 1251 ms.
    w.run_for(ms(500));
    assert_eq!(view(&w, 0), vec![Down, Down]);
    assert_eq!(view(&w, 1), vec![Down, Up]);
    assert_eq!(view(&w, 2), vec![Down, Up]);
    assert_eq!(deliveries_between(&w, N0, N1, cut_at), 0);
    assert_eq!(deliveries_between(&w, N0, N2, cut_at), 0);
    assert!(
        deliveries_between(&w, N1, N2, cut_at) > 0,
        "the other side keeps talking"
    );
    assert!(w.net.stats().partitioned >= 4, "the in-flight beats were dropped");
    let down_lines: Vec<String> = w
        .trace()
        .lines()
        .iter()
        .filter(|l| l.contains("peer down"))
        .cloned()
        .collect();
    assert_eq!(
        down_lines,
        vec![
            format!("{} n0 peer down n1", at(1251)),
            format!("{} n0 peer down n2", at(1251)),
            format!("{} n1 peer down n0", at(1251)),
            format!("{} n2 peer down n0", at(1251)),
        ]
    );

    w.net.heal_all();
    w.run_for(ms(200));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id} after heal");
    }
    assert_eq!(w.heartbeat(0).stats().downs, 2);
    assert_eq!(w.heartbeat(1).stats().downs, 1);
}

#[test]
fn an_asymmetric_cut_makes_two_nodes_disagree() {
    let mut w = cluster(NetConfig::reliable(ms(1)));
    w.run_for(Duration::from_secs(1));
    w.net.cut(N0, N1);
    w.run_for(Duration::from_secs(1));
    // n1 no longer hears n0. n0 still hears n1, so it believes all is well.
    assert_eq!(view(&w, 0), vec![Up, Up]);
    assert_eq!(view(&w, 1), vec![Down, Up]);
    assert_eq!(view(&w, 2), vec![Up, Up]);
}

#[test]
fn a_bridge_partition_is_partial() {
    let mut w = cluster(NetConfig::reliable(ms(1)));
    w.run_for(Duration::from_secs(1));
    w.net.partition(N0, N2);
    w.run_for(Duration::from_secs(1));
    // n1 is the bridge: it sees everyone, and everyone sees it.
    assert_eq!(view(&w, 0), vec![Up, Down]);
    assert_eq!(view(&w, 1), vec![Up, Up]);
    assert_eq!(view(&w, 2), vec![Down, Up]);
}

#[test]
fn a_message_in_flight_when_the_link_is_cut_is_lost() {
    let mut w = cluster(NetConfig::reliable(ms(10)));
    // Let view 0 start, so that only beats are on the wire.
    w.run_for(Duration::from_secs(1));
    assert!(w.converged());
    w.run_for(ms(5));
    assert_eq!(w.net.in_flight(), 6, "a round of beats is on the wire");
    let before = w.net.stats();
    let lines = w.trace().lines().len();
    w.net.split(&[N0], &[N1, N2]);
    w.run_for(ms(10));
    assert_eq!(w.net.in_flight(), 0);
    let s = w.net.stats();
    assert_eq!(
        s.partitioned - before.partitioned,
        4,
        "n0<->n1 and n0<->n2, both ways"
    );
    assert_eq!(s.delivered - before.delivered, 2, "n1<->n2 arrived");
    let dropped: Vec<String> = w.trace().lines()[lines..]
        .iter()
        .filter(|l| l.contains("net dropped"))
        .cloned()
        .collect();
    assert_eq!(dropped.len(), 4);
    assert!(dropped[0].ends_with("partition"), "{}", dropped[0]);
    assert!(
        dropped[0].contains(" beat boot=0 seq=11 view=0 "),
        "{}",
        dropped[0]
    );
}

#[test]
#[should_panic(expected = "never sent")]
fn the_network_never_delivers_what_nobody_sent() {
    // Take a real delivery event out of one world and drop it into another
    // world's queue. The second world's ledger has never seen the id.
    let mut source = cluster(NetConfig::reliable(ms(1)));
    let forged = loop {
        let fired = source.scheduler.pop().expect("a cluster always has events");
        if matches!(fired.event, Event::Deliver { .. }) {
            break fired.event;
        }
    };
    // A one-host world sends no heartbeats, so its ledger is empty.
    let mut victim = World::new(Disk::default());
    victim.scheduler.schedule_at(victim.now(), forged);
    victim.step();
}

#[test]
#[should_panic(expected = "never sent")]
fn the_same_copy_cannot_arrive_twice() {
    let mut w = cluster(NetConfig::reliable(ms(1)));
    let fired = loop {
        let fired = w.scheduler.pop().unwrap();
        if matches!(fired.event, Event::Deliver { .. }) {
            break fired;
        }
    };
    w.scheduler.schedule_at(fired.at, fired.event.clone());
    w.scheduler.schedule_at(fired.at, fired.event);
    // The first extra copy arrives fine. The second has no copy left in the
    // ledger to account for it.
    w.run_for(ms(1));
}

#[test]
fn duplicates_and_stragglers_cause_no_false_suspicion() {
    // Heavy duplication and reordering, no loss. A beat is at most 205 ms
    // late and the next one is at least 101 ms behind it, so the longest
    // silence is 304 ms. That is under `suspect_after`, so no gap between
    // beats is long enough to look like a crash.
    let mut w = cluster(NetConfig {
        latency_min: ms(1),
        latency_max: ms(5),
        loss: Chance::NEVER,
        duplicate: Chance::percent(30),
        reorder: Chance::percent(50),
        reorder_delay_max: ms(200),
    });
    w.run_for(Duration::from_secs(10));
    let s = w.net.stats();
    assert!(s.duplicated > 50 && s.reordered > 50, "{s:?}");
    for host in &w.hosts {
        let hs = host.heartbeat().stats();
        assert!(
            hs.duplicates > 0 && hs.reordered > 0,
            "node {} saw {hs:?}",
            host.id
        );
        assert_eq!(hs.downs, 0, "node {} flapped: {hs:?}", host.id);
        assert_eq!(view(&w, host.id), vec![Up, Up]);
    }
}

#[test]
fn heavy_loss_flaps_and_a_quiet_network_recovers() {
    let mut w = cluster(NetConfig {
        loss: Chance::percent(70),
        ..NetConfig::reliable(ms(1))
    });
    w.run_for(Duration::from_secs(10));
    let downs: u64 = w.hosts.iter().map(|h| h.heartbeat().stats().downs).sum();
    assert!(downs > 0, "70% loss never produced a false suspicion");
    // Faults off: the liveness check. Everyone must be Up within one
    // suspicion window plus one beat.
    w.net.set_config(NetConfig::reliable(ms(1)));
    w.run_for(ms(350 + 100 + 1));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
}

/// Three hosts and two clients under a random fault configuration and
/// random partitions, all from the seed. Then the faults stop, and the
/// cluster must converge. The trace must be a pure function of the seed.
fn chaos(prng: &mut Prng, trace: &mut determinism::Trace) {
    let config = WorldConfig {
        seed: prng.next_u64(),
        net: NetConfig::random(prng),
        partitions: Some(PartitionSchedule {
            gap_max: Duration::from_secs(2),
            hold_max: Duration::from_secs(1),
        }),
        ..WorldConfig::default()
    };
    let mut w = World::cluster(3, config);
    w.add_client(1, Config::default(), 0);
    w.add_client(2, Config::default(), prng.below(3) as NodeId);
    for i in 0..30u64 {
        let id = 1 + prng.below(2);
        let request = if prng.chance(1, 2) {
            Request::Put {
                key: b"k".into(),
                value: format!("v{i}").into_bytes(),
            }
        } else {
            Request::Get { key: b"k".into() }
        };
        w.submit(id, request);
        assert!(w.run_while(|w| !w.clients[&id].is_idle()));
    }
    w.run_for(Duration::from_secs(5));
    let chaos_stats = (w.stats, w.net.stats());
    assert!(
        w.stats.partitions > 0,
        "five seconds with a two-second gap always partitions"
    );

    // The quiet period.
    w.stop_partitions();
    w.net.set_config(NetConfig::reliable(ms(1)));
    w.run_for(ms(350 + 100 + 1));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id} did not recover");
    }
    assert!(
        w.run_until_converged(Duration::from_secs(5)),
        "the cluster did not settle"
    );
    assert!(
        w.net.in_flight() <= 6,
        "only the current round of beats is on the wire: {}",
        w.net.in_flight()
    );
    for line in w.trace().lines() {
        trace.record(format_args!("{line}"));
    }
    determinism::trace!(
        trace,
        "chaos {chaos_stats:?} completed={} gave_up={}",
        w.completed.len(),
        w.gave_up.len()
    );
}

#[test]
fn chaos_is_a_pure_function_of_the_seed() {
    for seed in 0..20 {
        check_determinism(seed, 3, chaos).unwrap_or_else(|d| panic!("{d}"));
    }
}

#[test]
fn known_cluster_fingerprint() {
    // Pins seed 1 of the chaos run. Any change to a trace line in the
    // node, client, heartbeat, scheduler, network model or disk model
    // changes it. Module 7 changed it on purpose: the configuration of the
    // run heads every trace. Module 9 changed it again: the three hosts
    // are one replicated store.
    let hash = check_determinism(1, 2, chaos).unwrap();
    assert_eq!(
        hash, 0x27ac_c9ca_e845_78d6,
        "seed 1 chaos hash changed: {hash:#018x}"
    );
}
