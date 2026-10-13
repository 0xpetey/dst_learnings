//! Module 7 acceptance tests: hosts stall, clocks drift and jump, the code
//! under test misbehaves at marked points, and every seed picks its own
//! subset of faults.
//!
//! Three things must hold whatever the faults:
//!
//! 1. A paused or clock-skewed host never loses a client's request. The
//!    request waits, or the client retries, and the answer is right.
//! 2. A fault point fires only when its site is enabled, and the system
//!    survives every site.
//! 3. When the faults stop, every host that can come back comes back, and
//!    every host sees every other as up. That is the liveness check.

use determinism::{check_determinism, Chance, Prng};
use kv_core::client::Config;
use kv_core::heartbeat::Liveness::{self, Down, Up};
use kv_core::{Duration, NodeId, Request, Response, Site};
use kv_env::fake::{BuggifyConfig, World, WorldConfig};
use sim::{ClockConfig, Skew};
use std::collections::{BTreeMap, BTreeSet};

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
fn view(w: &World, node: NodeId) -> Vec<Liveness> {
    let hb = w.heartbeat(node);
    hb.peers().map(|p| hb.liveness(p)).collect()
}
fn lines_containing(w: &World, needle: &str) -> Vec<String> {
    w.trace()
        .lines()
        .iter()
        .filter(|l| l.contains(needle))
        .cloned()
        .collect()
}

// ---- pauses ----------------------------------------------------------------

#[test]
fn a_paused_host_answers_nothing_until_it_resumes() {
    let mut w = World::cluster(1, WorldConfig::default());
    w.add_client(1, cfg(), 0);
    assert_eq!(w.call(1, put("a", "1")), Response::PutOk);
    w.pause(0);
    assert!(w.is_paused(0) && w.is_up(0));
    let rid = w.submit(1, get("a"));
    w.run_for(ms(250));
    assert!(w.take_completed(1, rid).is_none());
    // The put, then the get and its retry at 100 ms. The next retry is
    // due at 300 ms.
    assert_eq!(
        w.stats.requests_delivered, 3,
        "the get and one retry reached the doorstep"
    );
    let process = w.hosts[0].process();
    assert_eq!(process.driver.network.inbox_len(), 2, "and wait there unread");
    assert_eq!(w.node(0).stats().requests, 1, "the node saw only the put");
    w.resume(0);
    w.run_until_idle();
    assert_eq!(
        w.take_completed(1, rid),
        Some(Response::Value(Some(b"1".to_vec())))
    );
    let stats = w.node(0).stats();
    assert_eq!(stats.requests, 3, "both copies, read at once");
    assert_eq!(stats.duplicates, 1);
    assert_eq!((w.stats.pauses, w.stats.resumes), (1, 1));
    assert_eq!(w.hosts[0].incarnation(), 1, "a pause is not a restart");
}

#[test]
fn a_disk_completion_during_a_pause_waits_for_the_process() {
    let mut w = World::cluster(1, WorldConfig::default());
    w.add_client(1, cfg(), 0);
    let rid = w.submit(1, put("a", "1"));
    w.run_for(ms(2));
    assert_eq!(
        w.disk(0).pending(),
        2,
        "the superblock and the append are on the disk"
    );
    w.pause(0);
    w.run_for(ms(50));
    assert_eq!(w.disk(0).pending(), 0, "the device finished anyway");
    assert_eq!(w.hosts[0].process().driver.storage.completions_waiting(), 2);
    assert_eq!(w.node(0).pending_writes(), 1, "the node does not know yet");
    assert!(w.take_completed(1, rid).is_none());
    w.resume(0);
    w.run_until_idle();
    assert_eq!(w.take_completed(1, rid), Some(Response::PutOk));
    assert_eq!(w.node(0).log_seq(), 1);
}

#[test]
fn a_pause_looks_like_a_crash_to_the_peers_and_like_nothing_to_the_host() {
    let mut w = World::cluster(3, WorldConfig::default());
    w.run_for(Duration::from_secs(1));
    w.pause(0);
    w.run_for(ms(500));
    assert_eq!(view(&w, 1), vec![Down, Up]);
    assert_eq!(view(&w, 2), vec![Down, Up]);
    assert_eq!(view(&w, 0), vec![Up, Up], "frozen with its last view");
    assert!(w.stats.held_messages >= 8, "{}", w.stats.held_messages);
    assert_eq!(w.stats.dropped_at_down_hosts, 0, "nothing was dropped");
    w.resume(0);
    // On waking, the timer fires first: 500 ms of silence on node 0's own
    // clock, and both peers are suspect. Then the held beats arrive and
    // both are back. From the inside, the whole cluster vanished and
    // returned in one instant.
    assert_eq!(view(&w, 0), vec![Up, Up]);
    assert_eq!(w.heartbeat(0).stats().downs, 2, "both peers suspected on waking");
    assert_eq!(w.heartbeat(0).stats().ups, 4);
    w.run_for(ms(200));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
    // The paused node's beats stopped and started again, with no gap in
    // the sequence, so its peers saw no straggler and no duplicate.
    assert_eq!(w.heartbeat(1).stats().reordered, 0);
    assert_eq!(w.heartbeat(1).stats().downs, 1);
    let resume = lines_containing(&w, "resume");
    assert_eq!(resume.len(), 1);
    assert!(resume[0].contains("held="), "{}", resume[0]);
}

#[test]
fn a_crash_during_a_pause_ends_the_pause() {
    let mut w = World::cluster(2, WorldConfig::default());
    w.run_for(ms(100));
    w.pause(0);
    w.crash(0);
    assert!(!w.is_paused(0) && !w.is_up(0));
    w.restart(0);
    w.run_for(ms(200));
    assert!(!w.is_paused(0) && w.is_up(0));
    assert_eq!(view(&w, 0), vec![Up]);
}

#[test]
fn the_pause_schedule_stalls_one_host_at_a_time_and_stop_pauses_frees_it() {
    let mut w = World::cluster(
        3,
        WorldConfig {
            seed: 3,
            pauses: Some(kv_env::fake::PauseSchedule {
                gap_max: ms(300),
                hold_max: ms(400),
            }),
            ..WorldConfig::default()
        },
    );
    let mut max_paused = 0;
    for _ in 0..200 {
        w.run_for(ms(25));
        let paused = w.hosts.iter().filter(|h| h.is_paused()).count();
        max_paused = max_paused.max(paused);
    }
    assert_eq!(max_paused, 1);
    assert!(w.stats.pauses >= 5, "{:?}", w.stats);
    assert_eq!(
        w.stats.pauses - w.stats.resumes,
        w.hosts.iter().filter(|h| h.is_paused()).count() as u64
    );
    assert!(
        w.heartbeat(0).stats().downs + w.heartbeat(1).stats().downs > 0,
        "pauses were noticed"
    );
    w.stop_pauses();
    assert!(w.hosts.iter().all(|h| !h.is_paused()));
    w.run_for(ms(350 + 100 + 1));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
}

// ---- clocks ----------------------------------------------------------------

#[test]
fn a_clock_that_jumps_forward_suspects_every_peer_at_once() {
    let mut w = World::cluster(3, WorldConfig::default());
    w.run_for(Duration::from_secs(1));
    assert_eq!(w.local_now(0), w.now());
    w.jump_clock(0, ms(400));
    assert_eq!(w.local_now(0), w.now() + ms(400));
    // 400 ms of silence on node 0's clock, which is more than suspect_after.
    assert_eq!(view(&w, 0), vec![Down, Down]);
    assert_eq!(w.heartbeat(0).stats().downs, 2);
    assert_eq!(view(&w, 1), vec![Up, Up], "nothing happened to anyone else");
    w.run_for(ms(200));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
    assert_eq!(w.stats.clock_jumps, 1);
    let jump = lines_containing(&w, "clock jump");
    assert_eq!(jump.len(), 1);
    assert!(
        jump[0].ends_with("n0 clock jump by=400ms local=t=1400000000ns"),
        "{}",
        jump[0]
    );
}

#[test]
fn a_jump_smaller_than_the_margin_is_harmless() {
    let mut w = World::cluster(3, WorldConfig::default());
    w.run_for(Duration::from_secs(1));
    w.jump_clock(0, ms(200));
    assert_eq!(view(&w, 0), vec![Up, Up]);
    w.run_for(Duration::from_secs(2));
    for id in 0..3 {
        assert_eq!(w.heartbeat(id).stats().downs, 0, "node {id}");
    }
}

#[test]
fn a_fast_clock_beats_more_often_and_a_very_fast_one_suspects_its_peers() {
    let mut w = World::cluster(2, WorldConfig::default());
    // 20% fast: 1 s on the wall is 1.2 s on node 0's clock.
    w.hosts[0].clock.set_drift_ppm(200_000);
    w.run_for(Duration::from_secs(10));
    let fast = w.heartbeat(0).stats().sent;
    let normal = w.heartbeat(1).stats().sent;
    assert_eq!(normal, 101);
    assert!((118..=122).contains(&fast), "{fast}");
    assert_eq!(
        w.heartbeat(0).stats().downs,
        0,
        "350 ms is 292 ms of wall time, still above 100 ms"
    );
    assert_eq!(w.heartbeat(1).stats().downs, 0);
    assert_eq!(w.local_now(0), w.now() + Duration::from_secs(2));

    // 50% fast, the edge of the model: suspect_after is 233 ms of wall
    // time. The peer's beats are 100 ms apart and still arrive in time.
    w.hosts[0].clock.set_drift_ppm(500_000);
    w.run_for(Duration::from_secs(10));
    assert_eq!(w.heartbeat(0).stats().downs, 0);

    // Give the peer a slow clock too: its beats are now 200 ms of wall time
    // apart. Still under 233 ms. Drift alone does not eat a 3.5x margin.
    w.hosts[1].clock.set_drift_ppm(-500_000);
    w.run_for(Duration::from_secs(10));
    assert_eq!(w.heartbeat(0).stats().downs, 0);
    assert_eq!(w.heartbeat(1).stats().downs, 0);
}

#[test]
fn skewed_clocks_do_not_change_what_clients_see() {
    let mut w = World::cluster(
        3,
        WorldConfig {
            seed: 11,
            clocks: Some(ClockConfig {
                offset_max: Duration::from_secs(5),
                drift_ppm_max: 100_000,
                jump_gap_max: ms(500),
                jump_max: ms(300),
            }),
            ..WorldConfig::default()
        },
    );
    let skews: Vec<Skew> = w.hosts.iter().map(|h| h.clock.skew()).collect();
    assert!(skews.iter().any(|s| s.offset_nanos > 0));
    assert!(skews.iter().any(|s| s.drift_ppm != 0));
    assert!(skews.iter().all(|s| s.drift_ppm.abs() <= 100_000));
    for node in 0..3 {
        w.add_client(u64::from(node) + 1, cfg(), node);
    }
    for i in 0..30u64 {
        let id = i % 3 + 1;
        assert_eq!(w.call(id, put("k", &i.to_string())), Response::PutOk);
        assert_eq!(
            w.call(id, get("k")),
            Response::Value(Some(i.to_string().into_bytes()))
        );
    }
    assert!(w.stats.clock_jumps > 0, "{:?}", w.stats);
    assert!(w.gave_up.is_empty());
    let drawn = lines_containing(&w, "config drawn:");
    assert_eq!(drawn.len(), 1);
    assert!(
        drawn[0].contains("n0=[offset=") && drawn[0].contains("drift="),
        "{}",
        drawn[0]
    );
    w.stop_clock_jumps();
    w.run_for(ms(350 + 100 + 1));
    for id in 0..3 {
        assert_eq!(view(&w, id), vec![Up, Up], "node {id}");
    }
}

// ---- buggify ---------------------------------------------------------------

#[test]
fn fault_points_fire_only_when_enabled() {
    let quiet = World::cluster(1, WorldConfig::default());
    assert!(quiet.buggify_sites().is_empty());
    let mut w = World::cluster(
        1,
        WorldConfig {
            seed: 2,
            buggify: Some(BuggifyConfig {
                enable: Chance::ALWAYS,
                fire: Chance::percent(30),
            }),
            ..WorldConfig::default()
        },
    );
    assert_eq!(w.buggify_sites(), &Site::ALL);
    w.add_client(1, cfg(), 0);
    let mut model = BTreeMap::new();
    for i in 0..60u64 {
        let key = format!("k{}", i % 5);
        if i % 3 == 0 {
            assert_eq!(w.call(1, get(&key)), Response::Value(model.get(&key).cloned()));
        } else {
            let value = format!("v{i}").into_bytes();
            model.insert(key.clone(), value.clone());
            assert_eq!(
                w.call(
                    1,
                    Request::Put {
                        key: key.into(),
                        value
                    }
                ),
                Response::PutOk
            );
        }
    }
    let b = w.node(0).buggify();
    assert!(b.fired(Site::DropRequest) > 0);
    assert!(b.fired(Site::DropReply) > 0);
    assert!(b.fired(Site::DuplicateReply) > 0);
    assert_eq!(
        b.fired(Site::SkipBeat),
        0,
        "the node has no beats; the heartbeat has"
    );
    assert_eq!(
        w.stats.buggify_fires,
        b.total_fired() + w.heartbeat(0).buggify().total_fired() + w.replica(0).buggify().total_fired(),
        "every fire is in the trace"
    );
    assert_eq!(
        lines_containing(&w, "buggify drop_reply").len() as u64,
        b.fired(Site::DropReply)
    );
    assert!(
        w.clients[&1].retries() > 0,
        "every dropped request or reply cost a retry"
    );
    assert_eq!(
        w.node(0).stats().applied as usize,
        40,
        "each put applied exactly once"
    );
    let header = lines_containing(&w, "config drawn:");
    assert!(
        header[0].ends_with("buggify_sites=[drop_request drop_reply duplicate_reply skip_beat drop_prepare]"),
        "{}",
        header[0]
    );

    // A quiet world has no fault points at all.
    let mut w = World::cluster(1, WorldConfig::default());
    w.add_client(1, cfg(), 0);
    for _ in 0..20 {
        w.call(1, put("k", "v"));
    }
    assert_eq!(w.stats.buggify_fires, 0);
    assert!(lines_containing(&w, "buggify")
        .iter()
        .all(|l| l.contains("config")));
}

#[test]
fn skipped_beats_are_tolerated_by_the_peers() {
    let mut w = World::cluster(
        3,
        WorldConfig {
            seed: 4,
            buggify: Some(BuggifyConfig {
                enable: Chance::ALWAYS,
                fire: Chance::percent(50),
            }),
            ..WorldConfig::default()
        },
    );
    w.run_for(Duration::from_secs(20));
    let skipped: u64 = (0..3)
        .map(|n| w.heartbeat(n).buggify().fired(Site::SkipBeat))
        .sum();
    assert!(skipped > 200, "{skipped}");
    // Half the beats skipped at random: three in a row is a one in eight
    // event, and the margin is three and a half intervals. Some runs of
    // silence cross it, and the detector must come back every time.
    let downs: u64 = (0..3).map(|n| w.heartbeat(n).stats().downs).sum();
    let ups: u64 = (0..3).map(|n| w.heartbeat(n).stats().ups).sum();
    let down_now: u64 = (0..3)
        .map(|n| view(&w, n).iter().filter(|l| **l == Down).count() as u64)
        .sum();
    assert!(downs > 0, "the gaps were noticed");
    assert_eq!(
        ups + down_now,
        downs + 6,
        "and every one of them healed, or is about to"
    );
    for n in 0..3 {
        assert_eq!(
            w.heartbeat(n).stats().reordered,
            0,
            "node {n}: dense numbering, no stragglers"
        );
    }
}

// ---- swarm -----------------------------------------------------------------

#[test]
fn the_swarm_is_a_pure_function_of_the_seed() {
    for seed in 0..10 {
        assert_eq!(
            WorldConfig::swarm(&mut Prng::from_seed(seed)),
            WorldConfig::swarm(&mut Prng::from_seed(seed))
        );
    }
}

#[test]
fn every_family_is_on_in_some_runs_and_off_in_others() {
    let mut on = [0u32; 8];
    let mut subsets = BTreeSet::new();
    for seed in 0..64 {
        let config = WorldConfig::swarm(&mut Prng::from_seed(seed));
        let families = config.families();
        subsets.insert(families);
        for (count, enabled) in on.iter_mut().zip(families) {
            *count += u32::from(enabled);
        }
    }
    for (i, n) in on.iter().enumerate() {
        assert!((16..=48).contains(n), "family {i} on in {n} of 64 runs");
    }
    assert!(subsets.len() >= 40, "{} distinct subsets of 64", subsets.len());
    assert!(
        !subsets.contains(&[true; 8]) || subsets.len() > 1,
        "everything-on is one configuration among many"
    );
}

#[test]
fn the_configuration_heads_every_trace() {
    let config = WorldConfig::swarm(&mut Prng::from_seed(5));
    let w = World::cluster(3, config.clone());
    let lines: Vec<String> = w.trace().lines().to_vec();
    assert!(lines[0].starts_with("t=0ns config seed="), "{}", lines[0]);
    assert_eq!(lines[0], format!("t=0ns config {config}"));
    assert!(lines[1].starts_with("t=0ns config drawn:"), "{}", lines[1]);
    for name in [
        "net=",
        "disk=",
        "partitions=",
        "crashes=",
        "pauses=",
        "clocks=",
        "buggify=",
        "slow_host=",
    ] {
        assert!(lines[0].contains(name), "{name} missing from {}", lines[0]);
    }
    if let Some(n) = w.slow_host() {
        assert!(lines[1].contains(&format!("slow_host=n{n}")));
        let factor = config.slow_host.as_ref().unwrap().factor;
        assert_eq!(
            w.disk(n).config().write_latency_min,
            config.disk.write_latency_min * factor
        );
    }
}

/// Three hosts, one client each, under whatever faults the seed chose.
/// Then the faults stop and the cluster must come back. The trace is a
/// pure function of the seed.
fn chaos(prng: &mut Prng, trace: &mut determinism::Trace) {
    let config = WorldConfig::swarm(prng);
    let hb = config.heartbeat;
    let mut w = World::cluster(3, config);
    for node in 0..3 {
        w.add_client(u64::from(node) + 1, cfg(), node);
    }
    let keys: Vec<Vec<u8>> = (0..4).map(|i| format!("k{i}").into_bytes()).collect();
    let mut completed = 0;
    for i in 0..100u64 {
        let node = prng.below(3) as NodeId;
        let id = u64::from(node) + 1;
        let key = prng.pick(&keys).clone();
        let request = match prng.below(3) {
            0 => Request::Get { key },
            1 => Request::Put {
                key,
                value: format!("v{i}").into_bytes(),
            },
            _ => Request::Delete { key },
        };
        let rid = w.submit(id, request);
        assert!(w.run_while(|w| !w.clients[&id].is_idle()));
        if w.take_completed(id, rid).is_some() {
            completed += 1;
        } else {
            assert!(w.gave_up.contains(&(id, rid)));
        }
    }

    // The quiet period: every fault off, then the liveness check. Since
    // Module 9 a damaged log is repaired from a peer, so every host comes
    // back, and the cluster must settle on one view with one log.
    w.stop_faults();
    w.run_until_idle();
    w.run_for(hb.suspect_after + hb.interval + ms(1));
    for host in &w.hosts {
        assert!(!host.is_paused(), "n{} still paused after stop_faults", host.id);
        assert!(host.is_up(), "n{} did not come back", host.id);
    }
    assert!(
        w.run_until_converged(Duration::from_secs(10)),
        "the cluster did not settle"
    );
    for a in 0..3 {
        let hb = w.heartbeat(a);
        for b in 0..3 {
            if a != b {
                assert_eq!(hb.liveness(b), Up, "node {a} does not see node {b}");
            }
        }
    }
    // Every host serves, through the primary. A fresh client, because the
    // old ids name sessions that may be in the log.
    for n in 0..3 {
        let id = 100 + u64::from(n);
        w.add_client(id, cfg(), n);
        assert!(matches!(w.call(id, get("k0")), Response::Value(_)));
    }
    let repairs: u64 = (0..3).map(|n| w.replica(n).stats().repairs).sum();
    for line in w.trace().lines() {
        trace.record(format_args!("{line}"));
    }
    determinism::trace!(
        trace,
        "swarm {:?} completed={completed} gave_up={} repairs={repairs} primary={:?}",
        w.stats,
        w.gave_up.len(),
        w.primary()
    );
}

#[test]
fn swarm_chaos_is_a_pure_function_of_the_seed_and_recovers_in_the_quiet_period() {
    for seed in 0..24 {
        check_determinism(seed, 3, chaos).unwrap_or_else(|d| panic!("{d}"));
    }
}

#[test]
fn known_swarm_fingerprint() {
    // Pins seed 1 of the swarm run. Any change to a trace line in the
    // node, client, heartbeat, scheduler, network, disk, clock, buggify
    // sites or world changes it. Module 9 changed it: the three hosts are
    // one replicated store, and `drop_prepare` is a fifth site.
    let hash = check_determinism(1, 2, chaos).unwrap();
    assert_eq!(
        hash, 0x8697_e36c_2983_8b8b,
        "seed 1 swarm hash changed: {hash:#018x}"
    );
}
