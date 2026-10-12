//! Module 8 acceptance tests: a seed-driven workload with many clients at
//! once, a reference model, a linearizability checker, and three planted
//! bugs that the checker must find.
//!
//! Three things must hold whatever the faults:
//!
//! 1. Every history the store produces is linearizable: each operation
//!    took effect at some instant between its call and its return, in an
//!    order the model agrees with.
//! 2. The node's map is the log replayed, whenever the node is idle.
//! 3. Once the faults stop, every request completes.

use checker::kv::{self, KvModel};
use checker::linearize::Verdict;
use checker::model::{check_sequential, SequentialError};
use checker::{Step, Workload, WorkloadConfig};
use determinism::{check_determinism, Chance, Prng};
use kv_core::client::Config;
use kv_core::log;
use kv_core::{Bug, Duration, NodeId, Request};
use kv_env::fake::{CrashSchedule, World, WorldConfig};
use sim::NetConfig;
use std::collections::BTreeSet;

/// Candidates per key. The runs below never get near it.
const LIMIT: u64 = 2_000_000;

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
/// Clients that never pause: the number in flight is the number of clients.
fn busy(hot_keys: u32) -> WorkloadConfig {
    WorkloadConfig {
        keys: 8,
        hot_keys,
        hot: if hot_keys == 0 {
            Chance::NEVER
        } else {
            Chance::percent(70)
        },
        think: Chance::NEVER,
        ..WorkloadConfig::default()
    }
}
/// The node's map, as the checker sees it.
fn entries(w: &World, node: NodeId) -> Vec<(Vec<u8>, Vec<u8>)> {
    w.node(node)
        .entries()
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect()
}
/// Think lines in the trace: one per pause between operations.
trait Thinks {
    fn stats_thinks(&self) -> u64;
}
impl Thinks for World {
    fn stats_thinks(&self) -> u64 {
        self.trace()
            .lines()
            .iter()
            .filter(|l| l.contains(" think for="))
            .count() as u64
    }
}
/// The log on `node`'s disk, replayed.
fn replayed(w: &World, node: NodeId) -> Vec<(Vec<u8>, Vec<u8>)> {
    let recovered = log::recover(w.disk(node).medium());
    kv::replay(&recovered.records)
        .entries()
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect()
}

// ---- the generator ----------------------------------------------------------

#[test]
fn the_workload_is_a_pure_function_of_the_seed() {
    for seed in 0..8 {
        let a = WorkloadConfig::random(&mut Prng::from_seed(seed));
        let b = WorkloadConfig::random(&mut Prng::from_seed(seed));
        assert_eq!(a, b);
        a.validate();
        let mut wa = Workload::new(a.clone(), Prng::from_seed(seed));
        let mut wb = Workload::new(a, Prng::from_seed(seed));
        for i in 0..500u64 {
            assert_eq!(wa.next(1 + i % 3), wb.next(1 + i % 3));
        }
        assert_eq!(wa.stats(), wb.stats());
    }
}

#[test]
fn hot_keys_are_hot_the_mix_follows_the_weights_and_values_are_unique() {
    let mut w = Workload::new(WorkloadConfig::default(), Prng::from_seed(3));
    let mut values = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for i in 0..10_000u64 {
        match w.next(1 + i % 4) {
            Step::Request(Request::Put { key, value }) => {
                assert!(values.insert(value), "a value repeated");
                keys.insert(key);
            }
            Step::Request(r) => {
                keys.insert(r.key().to_vec());
            }
            Step::Think(d) => assert!(!d.is_zero() && d <= ms(100), "{d:?}"),
        }
    }
    let s = w.stats();
    assert_eq!(s.requests + s.thinks, 10_000);
    assert!((2_300..=2_700).contains(&s.thinks), "{s:?}");
    let pct = |n: u64| n * 100 / s.requests;
    assert!((37..=43).contains(&pct(s.gets)), "{s:?}");
    assert!((37..=43).contains(&pct(s.puts)), "{s:?}");
    assert!((17..=23).contains(&pct(s.deletes)), "{s:?}");
    // Half the operations go to the hot set on purpose, and one in eight
    // of the rest lands there by chance.
    assert!((53..=59).contains(&pct(s.hot)), "{s:?}");
    assert_eq!(keys.len(), 16, "every key was used");
    assert_eq!(values.len() as u64, s.puts);
    assert!(values.iter().all(|v| v.len() <= 16));
    assert!(
        values.iter().any(|v| v.ends_with(b"...")),
        "padded to a drawn length"
    );
}

// ---- sequential, then concurrent -------------------------------------------

#[test]
fn one_client_makes_a_sequential_history_and_the_model_is_the_whole_check() {
    let mut w = World::cluster(1, WorldConfig::default());
    w.add_client(1, cfg(), 0);
    let mut workload = Workload::new(busy(2), Prng::from_seed(7));
    let run = w.run_workload(&mut workload, 200);
    assert_eq!(run.submitted, 200);
    assert_eq!(run.completed, 200);
    assert!(w.history.is_sequential());
    assert_eq!(w.history.max_concurrency(), 1);
    let model = check_sequential::<KvModel>(&w.history).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(model.len(), w.node(0).len());
    // The search agrees, and never had a choice to make.
    let report = kv::check(&w.history, LIMIT);
    assert!(report.is_linearizable());
    assert_eq!(report.stats.steps as usize, w.history.len());
    assert!(w
        .trace()
        .lines()
        .iter()
        .any(|l| l.contains("workload clients=1 requests=200 keys=8 hot=2@70/100")));
}

#[test]
fn many_clients_overlap_and_only_the_search_can_judge_them() {
    let mut w = World::cluster(1, WorldConfig::default());
    for id in 1..=4 {
        w.add_client(id, cfg(), 0);
    }
    let mut workload = Workload::new(busy(2), Prng::from_seed(7));
    let run = w.run_workload(&mut workload, 300);
    assert_eq!(run.completed, 300);
    assert!(!w.history.is_sequential());
    assert_eq!(w.history.max_concurrency(), 4);
    assert!(matches!(
        check_sequential::<KvModel>(&w.history),
        Err(SequentialError::NotSequential { .. })
    ));
    let report = kv::check(&w.history, LIMIT);
    assert!(report.is_linearizable(), "{:?}", report.failed);
    assert_eq!(report.keys, 8);
    assert!(
        report.stats.steps as usize > w.history.len(),
        "with overlap the search has choices: {:?}",
        report.stats
    );
    // The white-box check: the map is the log replayed.
    assert_eq!(entries(&w, 0), replayed(&w, 0));
}

#[test]
fn thinking_clients_vary_the_number_in_flight() {
    let mut w = World::cluster(1, WorldConfig::default());
    for id in 1..=4 {
        w.add_client(id, cfg(), 0);
    }
    let config = WorkloadConfig {
        think: Chance::percent(50),
        think_max: ms(20),
        ..busy(2)
    };
    let mut workload = Workload::new(config, Prng::from_seed(1));
    let run = w.run_workload(&mut workload, 200);
    assert_eq!(run.completed, 200);
    let thinks = w.stats_thinks();
    assert!(thinks > 100, "{thinks}");
    assert_eq!(workload.stats().thinks, thinks);
    assert!(w.clients.values().all(kv_core::Client::is_idle));
    assert!(
        (1..=4).all(|id| !w.is_thinking(id)),
        "the run ends with everyone awake"
    );
    assert!(kv::check(&w.history, LIMIT).is_linearizable());
}

// ---- under faults ------------------------------------------------------------

/// Three hosts, two clients each, under whatever faults the seed chose.
/// The history must be linearizable, the primary's map must be its log,
/// and once the faults stop every request must complete. Since Module 9
/// the three hosts are one store, so there is one history to check.
fn chaos(prng: &mut Prng, trace: &mut determinism::Trace) {
    let config = WorldConfig::swarm(prng);
    let hb = config.heartbeat;
    let disk = config.disk.clone();
    let mut w = World::cluster(3, config);
    for node in 0..3u8 {
        for k in 0..2u64 {
            w.add_client(1 + u64::from(node) * 2 + k, cfg(), node);
        }
    }
    let workload_config = WorkloadConfig::random(prng);
    let mut workload = Workload::new(workload_config, prng.fork());
    let chaos_run = w.run_workload(&mut workload, 150);

    // The quiet period, and the liveness check: everything asked for
    // after the faults stop is answered.
    w.stop_faults();
    w.run_until_idle();
    w.run_for(hb.suspect_after + hb.interval + ms(1));
    assert!(
        w.run_until_converged(Duration::from_secs(10)),
        "the cluster did not settle"
    );
    let quiet_run = w.run_workload(&mut workload, 30);
    assert_eq!(
        quiet_run.gave_up, 0,
        "a request after the faults stopped was not answered"
    );
    assert_eq!(quiet_run.submitted, quiet_run.completed);

    // Safety: the clients' view is linearizable.
    let report = kv::check(&w.history, LIMIT);
    if let Some(failed) = &report.failed {
        panic!(
            "key {:?}: {}\n{:#?}",
            String::from_utf8_lossy(&failed.key),
            match &failed.verdict {
                Verdict::Illegal(i) => i.to_string(),
                v => format!("{v:?}"),
            },
            failed.history
        );
    }
    let steps = report.stats.steps;
    // And the white box: the primary's map is its log, on a disk that
    // keeps it, and every log is a prefix of every other.
    assert!(w.run_until_converged(Duration::from_secs(10)));
    w.check_logs()
        .unwrap_or_else(|(a, b)| panic!("logs of n{a} and n{b} diverged"));
    let primary = w.primary().expect("converged");
    let can_damage = !disk.lost_write.is_never()
        || !disk.corrupt_write.is_never()
        || !disk.corrupt_read.is_never()
        || !disk.misdirect_write.is_never();
    if !can_damage {
        assert_eq!(entries(&w, primary), replayed(&w, primary), "node {primary}");
    }
    for line in w.trace().lines() {
        trace.record(format_args!("{line}"));
    }
    determinism::trace!(
        trace,
        "workload {:?} chaos={chaos_run:?} quiet={quiet_run:?} history={} pending={} steps={steps} {:?}",
        workload.stats(),
        w.history.len(),
        w.history.pending(),
        w.stats
    );
}

#[test]
fn workload_chaos_is_linearizable_and_a_pure_function_of_the_seed() {
    for seed in 0..16 {
        check_determinism(seed, 2, chaos).unwrap_or_else(|d| panic!("{d}"));
    }
}

#[test]
fn known_workload_fingerprint() {
    // Pins seed 1 of the workload chaos run. Any change to a trace line in
    // the node, client, heartbeat, scheduler, network, disk, clock,
    // buggify sites, workload or world changes it. Module 9 changed it:
    // the three hosts are one replicated store.
    let hash = check_determinism(1, 2, chaos).unwrap();
    assert_eq!(
        hash, 0xe688_4dda_2f79_6c8c,
        "seed 1 workload hash changed: {hash:#018x}"
    );
}

// ---- the planted bugs ---------------------------------------------------------

/// One host, `clients` clients, a random network, `bug` planted. Returns
/// the seeds on which the checker caught it.
fn hunt(bug: Bug, clients: u64, seeds: std::ops::Range<u64>, crashes: bool, reliable: bool) -> Vec<u64> {
    let mut found = Vec::new();
    for seed in seeds {
        let mut prng = Prng::from_seed(seed);
        let config = WorldConfig {
            seed: prng.next_u64(),
            net: if reliable {
                NetConfig::default()
            } else {
                NetConfig::random(&mut prng)
            },
            crashes: crashes.then_some(CrashSchedule {
                gap_max: ms(400),
                down_max: ms(100),
            }),
            bugs: vec![bug],
            ..WorldConfig::default()
        };
        let mut w = World::cluster(1, config);
        for id in 1..=clients {
            w.add_client(id, cfg(), 0);
        }
        let mut workload = Workload::new(busy(2), prng.fork());
        w.run_workload(&mut workload, 120);
        let report = kv::check(&w.history, LIMIT);
        if let Some(failed) = report.failed {
            assert!(
                matches!(failed.verdict, Verdict::Illegal(_)),
                "seed {seed}: {:?}",
                failed.verdict
            );
            found.push(seed);
        }
    }
    found
}

#[test]
fn each_planted_bug_is_found_by_the_checker() {
    // The lost update needs no fault at all, only two clients on one key
    // at the same moment. A reliable network keeps the clients in step,
    // so that happens often. A random one spreads them out, and it
    // happens on one seed in twelve.
    let lost = hunt(Bug::LostUpdate, 3, 0..12, false, true);
    let stale = hunt(Bug::StaleReadAfterRestart, 3, 0..12, true, false);
    let dup = hunt(Bug::DuplicateApplyOnRetry, 3, 0..12, false, false);
    println!("lost update: seeds {lost:?}");
    println!("stale read after restart: seeds {stale:?}");
    println!("duplicate apply on retry: seeds {dup:?}");
    assert!(lost.len() >= 3, "lost update found on {lost:?}");
    assert!(stale.len() >= 3, "stale read found on {stale:?}");
    assert!(dup.len() >= 3, "duplicate apply found on {dup:?}");
}

#[test]
fn each_bug_hides_from_a_workload_that_cannot_reach_it() {
    // One client never has two writes in flight.
    assert!(hunt(Bug::LostUpdate, 1, 0..12, false, true).is_empty());
    // No crash, no restart, no recovery.
    assert!(hunt(Bug::StaleReadAfterRestart, 3, 0..12, false, false).is_empty());
    // A reliable network never loses a reply, so nothing is retried.
    assert!(hunt(Bug::DuplicateApplyOnRetry, 3, 0..12, false, true).is_empty());
}

#[test]
fn the_white_box_check_finds_a_bug_before_any_client_can() {
    // A lost update is in the log and not in the map. No read has to
    // happen for the replay to disagree.
    let mut caught = 0;
    for seed in 0..8 {
        let mut w = World::cluster(
            1,
            WorldConfig {
                seed,
                bugs: vec![Bug::LostUpdate],
                ..WorldConfig::default()
            },
        );
        for id in 1..=3 {
            w.add_client(id, cfg(), 0);
        }
        // Writes only, so that no read can see the difference.
        let config = WorkloadConfig {
            gets: 0,
            deletes: 0,
            ..busy(2)
        };
        let mut workload = Workload::new(config, Prng::from_seed(seed));
        w.run_workload(&mut workload, 60);
        assert!(kv::check(&w.history, LIMIT).is_linearizable(), "nothing was read");
        if entries(&w, 0) != replayed(&w, 0) {
            caught += 1;
        }
    }
    assert!(caught >= 4, "the replay disagreed on {caught} of 8 seeds");

    // A stale read after restart is caught by the replay at the restart,
    // before the first request reaches the new process.
    let mut w = World::cluster(
        1,
        WorldConfig {
            bugs: vec![Bug::StaleReadAfterRestart],
            ..WorldConfig::default()
        },
    );
    w.add_client(1, cfg(), 0);
    let mut workload = Workload::new(busy(2), Prng::from_seed(2));
    w.run_workload(&mut workload, 20);
    assert_eq!(entries(&w, 0), replayed(&w, 0));
    w.crash(0);
    w.restart(0);
    w.run_until_idle();
    assert_ne!(entries(&w, 0), replayed(&w, 0), "one write behind");
    assert_eq!(
        replayed(&w, 0),
        kv::replay(w.node(0).recovered_records())
            .entries()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect::<Vec<_>>(),
        "the records are all there; the map is not"
    );
}

#[test]
fn a_failing_check_names_the_operations() {
    // The first seed on which the duplicate apply is caught, as the
    // lesson shows it.
    let seed = hunt(Bug::DuplicateApplyOnRetry, 3, 0..12, false, false)[0];
    let mut prng = Prng::from_seed(seed);
    let config = WorldConfig {
        seed: prng.next_u64(),
        net: NetConfig::random(&mut prng),
        bugs: vec![Bug::DuplicateApplyOnRetry],
        ..WorldConfig::default()
    };
    let mut w = World::cluster(1, config);
    for id in 1..=3 {
        w.add_client(id, cfg(), 0);
    }
    let mut workload = Workload::new(busy(2), prng.fork());
    w.run_workload(&mut workload, 120);
    let failed = kv::check(&w.history, LIMIT).failed.expect("caught");
    let Verdict::Illegal(illegal) = &failed.verdict else {
        panic!("{:?}", failed.verdict)
    };
    println!(
        "seed {seed} key {:?}: {illegal}",
        String::from_utf8_lossy(&failed.key)
    );
    for (i, op) in failed.history.ops().iter().enumerate() {
        println!(
            "  {i:3} c{} id={} {:?} call={} ret={:?} -> {:?}",
            op.client, op.request_id, op.request, op.call, op.ret, op.response
        );
    }
    assert!(!illegal.stuck.is_empty());
    assert!(illegal.linearized.len() < failed.history.completed());
    assert!(w.node(0).stats().duplicates > 0, "a retry reached the node");
}
