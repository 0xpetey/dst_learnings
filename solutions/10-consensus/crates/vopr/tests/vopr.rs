//! Module 10 acceptance tests, part two: the simulator binary.
//!
//! Five things must hold:
//!
//! 1. A run is a pure function of its seed, in one process and across
//!    two.
//! 2. The swarm reaches the consensus code: views change, logs are
//!    pulled, damaged disks are repaired, and the core serves throughout.
//! 3. A planted bug in the view change is found, and so is a lease.
//! 4. Every seed that once found a bug still passes.
//! 5. Twenty thousand seeds pass. That one is run by hand.

use determinism::harness::run_process_twice;
use kv_core::{Bug, Duration};
use kv_env::fake::{CrashSchedule, PartitionSchedule};
use sim::ClockConfig;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use vopr::{run, RunConfig};

#[test]
fn a_run_is_a_pure_function_of_the_seed() {
    for seed in 0..8 {
        let config = RunConfig::from_seed(seed, 100);
        let a = run(&config);
        let b = run(&config);
        assert_eq!(a.hash, b.hash, "seed {seed}");
        assert_eq!(a.stats, b.stats, "seed {seed}");
        assert_eq!(a.replicas, b.replicas, "seed {seed}");
        assert_eq!(a.chaos, b.chaos);
        assert_eq!((a.chaos.submitted, a.quiet.completed), (100, 30));
    }
}

#[test]
fn the_seed_draws_the_cluster_size_and_the_core_last() {
    let mut fives = 0;
    let mut cores = 0;
    for seed in 0..64 {
        let config = RunConfig::from_seed(seed, 10);
        assert!(config.nodes == 3 || config.nodes == 5, "seed {seed}");
        fives += u64::from(config.nodes == 5);
        if !config.world.core.is_empty() {
            cores += 1;
            assert_eq!(
                config.world.core.len(),
                usize::from(config.nodes / 2 + 1),
                "a majority"
            );
            assert!(config.world.core.iter().all(|&n| n < config.nodes));
            assert!(config.to_string().contains(" core=[n"), "{config}");
        }
    }
    assert!((4..=28).contains(&fives), "{fives} of 64 seeds had five nodes");
    assert!((20..=44).contains(&cores), "{cores} of 64 seeds had a core");
}

#[test]
fn the_configuration_is_one_readable_line() {
    let config = RunConfig::from_seed(7, 150);
    let line = config.to_string();
    assert!(
        line.starts_with("seed=7 nodes=") && line.contains(" requests=150 quiet=30 seed="),
        "{line}"
    );
    assert!(
        line.contains(" net=[") && line.contains(" disk=[") && line.contains(" workload=[keys="),
        "{line}"
    );
    assert_eq!(RunConfig::from_seed(7, 150), config, "drawn the same way twice");
    assert_ne!(RunConfig::from_seed(8, 150).world, config.world);
}

#[test]
fn the_binary_agrees_with_itself_across_processes() {
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let hash = run_process_twice(bin, &["--requests", "60"], 3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        hash,
        run(&RunConfig::from_seed(3, 60)).hash,
        "the binary and the library agree"
    );
    let sweep = run_process_twice(bin, &["--requests", "40", "--seeds", "10..13"], 99)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(sweep, 0);
}

#[test]
fn known_vopr_fingerprint() {
    // Pins seed 1 of the full run. Any change to a trace line in the
    // replica, node, client, heartbeat, scheduler, network, disk, clock,
    // buggify sites, workload or world changes it.
    let hash = run(&RunConfig::from_seed(1, 150)).hash;
    assert_eq!(
        hash, 0xcc45_30a4_8c93_496b,
        "seed 1 vopr hash changed: {hash:#018x}"
    );
}

#[test]
fn the_swarm_reaches_the_consensus_code() {
    let mut promotions = 0;
    let mut pulls = 0;
    let mut repairs = 0;
    let mut step_downs = 0;
    let mut truncations = 0;
    let mut reads = 0;
    let mut seeds_with_failover = 0;
    let mut seeds_with_core = 0;
    let mut seeds_with_five = 0;
    for seed in 0..40 {
        let config = RunConfig::from_seed(seed, 100);
        let report = run(&config);
        let p: u64 = report.replicas.iter().map(|r| r.promotions).sum();
        promotions += p;
        seeds_with_failover += u64::from(p > 0);
        seeds_with_core += u64::from(!config.world.core.is_empty());
        seeds_with_five += u64::from(config.nodes == 5);
        pulls += report.replicas.iter().map(|r| r.pulls_sent).sum::<u64>();
        repairs += report.replicas.iter().map(|r| r.repairs).sum::<u64>();
        step_downs += report.replicas.iter().map(|r| r.step_downs).sum::<u64>();
        truncations += report.replicas.iter().map(|r| r.truncations).sum::<u64>();
        reads += report.replicas.iter().map(|r| r.reads).sum::<u64>();
        assert!(report.converged_in <= Duration::from_secs(10));
    }
    println!(
        "promotions={promotions} seeds_with_failover={seeds_with_failover} pulls={pulls} repairs={repairs} step_downs={step_downs} truncations={truncations} reads={reads} seeds_with_core={seeds_with_core} seeds_with_five={seeds_with_five}"
    );
    assert!(
        seeds_with_failover >= 10,
        "{seeds_with_failover} of 40 seeds failed over"
    );
    assert!(pulls > 0, "no new primary ever had to pull a log");
    assert!(repairs > 0, "no damaged log was ever repaired");
    assert!(step_downs > 0, "no primary ever stepped down");
    assert!(truncations > 0, "no uncommitted tail was ever dropped");
    assert!(reads > 500, "reads were not answered through the barrier");
    assert!(seeds_with_core >= 10 && seeds_with_five >= 2);
}

/// Run `config` and return the message of the check that fired, if one did.
fn failure(config: &RunConfig) -> Option<String> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = catch_unwind(AssertUnwindSafe(|| run(config)));
    std::panic::set_hook(hook);
    let err = outcome.err()?;
    Some(
        err.downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    )
}

#[test]
fn a_stale_primary_that_keeps_answering_reads_is_caught() {
    // The bug needs a view change with the old primary alive, so
    // partitions are forced on. Every failure must be a safety failure
    // named by the checker, and some seed must fail.
    let mut caught = Vec::new();
    for seed in 0..24 {
        let mut config = RunConfig::from_seed(seed, 120);
        config.world.bugs = vec![Bug::StaleReadAfterStepDown];
        config.world.partitions = Some(PartitionSchedule {
            gap_max: Duration::from_millis(800),
            hold_max: Duration::from_millis(1200),
        });
        config.world.crashes = None;
        if let Some(message) = failure(&config) {
            assert!(
                message.contains("not linearizable"),
                "seed {seed} failed for another reason: {message}"
            );
            caught.push(seed);
        }
    }
    println!("stale read after step down: seeds {caught:?}");
    assert!(caught.len() >= 3, "caught on {caught:?}");
    // Without the bug the same seeds pass.
    for &seed in caught.iter().take(3) {
        let mut config = RunConfig::from_seed(seed, 120);
        config.world.partitions = Some(PartitionSchedule {
            gap_max: Duration::from_millis(800),
            hold_max: Duration::from_millis(1200),
        });
        config.world.crashes = None;
        assert_eq!(failure(&config), None, "seed {seed}");
    }
}

#[test]
fn a_lease_is_only_as_good_as_the_clock() {
    // A primary that answers reads on a lease measured by its own clock.
    // With clocks that run slow and a partition to put a new primary in
    // charge, the lease outlives the view, and the reads are stale.
    // The chaos phase must outlast a few partitions, so these runs are
    // five times the usual length.
    let mut caught = Vec::new();
    for seed in 0..24 {
        let mut config = RunConfig::from_seed(seed, 600);
        config.nodes = 3;
        config.world.core = Vec::new();
        config.world.bugs = vec![Bug::LeaseRead];
        config.world.partitions = Some(PartitionSchedule {
            gap_max: Duration::from_millis(800),
            hold_max: Duration::from_millis(1200),
        });
        config.world.clocks = Some(ClockConfig {
            offset_max: Duration::from_millis(100),
            drift_ppm_max: 500_000,
            jump_gap_max: Duration::ZERO,
            jump_max: Duration::ZERO,
        });
        config.world.crashes = None;
        if let Some(message) = failure(&config) {
            assert!(
                message.contains("not linearizable"),
                "seed {seed} failed for another reason: {message}"
            );
            caught.push(seed);
        }
    }
    println!("lease read: seeds {caught:?}");
    assert!(!caught.is_empty(), "caught on {caught:?}");
    // The same seeds, with the barrier instead of the lease, pass.
    for &seed in caught.iter().take(3) {
        let mut config = RunConfig::from_seed(seed, 600);
        config.nodes = 3;
        config.world.core = Vec::new();
        config.world.partitions = Some(PartitionSchedule {
            gap_max: Duration::from_millis(800),
            hold_max: Duration::from_millis(1200),
        });
        config.world.clocks = Some(ClockConfig {
            offset_max: Duration::from_millis(100),
            drift_ppm_max: 500_000,
            jump_gap_max: Duration::ZERO,
            jump_max: Duration::ZERO,
        });
        config.world.crashes = None;
        assert_eq!(failure(&config), None, "seed {seed}");
    }
}

#[test]
fn the_core_is_served_throughout_the_chaos() {
    // Crashes, pauses and partitions for everyone outside the core, and
    // not one request from a core client given up on.
    let mut served = 0;
    for seed in 0..12 {
        let mut config = RunConfig::from_seed(seed, 100);
        config.nodes = 3;
        config.world.core = vec![0, 1];
        config.world.partitions = Some(PartitionSchedule {
            gap_max: Duration::from_millis(500),
            hold_max: Duration::from_millis(1500),
        });
        config.world.crashes = Some(CrashSchedule {
            gap_max: Duration::from_millis(500),
            down_max: Duration::from_millis(800),
        });
        let report = run(&config);
        served += report.chaos.completed;
        assert!(
            report.stats.partitions > 0 && report.stats.heals > 0,
            "seed {seed}"
        );
    }
    assert!(served >= 12 * 60, "{served} requests served");
}

#[test]
fn regression_seeds_still_pass() {
    // Seeds that found a bug while the module was written. Each one is a
    // run that once failed and now must not. The lesson tells their
    // stories.
    for seed in REGRESSION_SEEDS {
        let mut config = RunConfig::from_seed(seed, 150);
        config.nodes = 3;
        config.world.core = Vec::new();
        run(&config);
    }
}

/// The seeds that found the bugs the lesson's journal tells, in the order
/// they were found. The seed names a configuration under the draws of
/// the day it failed; the ones found before the cluster size and the
/// core were drawn are kept under three nodes and no core.
///
/// 7: a backup took the view before its log and offered a short log
/// under the view's number. 13: the guard's own bug, the commit number
/// adopted before the tail was dropped. 19: a truncation that forgot the
/// appends below the cut. 22: the catch-up target that moved with every
/// append. 24: a straggling beat from the primary that was shorter than
/// the prepares since. 36: a repaired log offered under the view of the
/// records before the hole. 525: a view started with a log shorter than
/// a commit the primary had heard. 562: a vote forgotten at a restart,
/// and two majorities. 563: an old durable count carried into a new
/// view. 7775: a cut-off primary serving a client that had moved on.
const REGRESSION_SEEDS: [u64; 10] = [7, 13, 19, 22, 24, 36, 525, 562, 563, 7775];
