//! Module 9 acceptance tests, part two: the simulator binary.
//!
//! Four things must hold:
//!
//! 1. A run is a pure function of its seed, in one process and across
//!    two.
//! 2. The swarm reaches the replication code: primaries fail over, logs
//!    are pulled, damaged disks are repaired.
//! 3. A planted bug in the replication code is found.
//! 4. Every seed that once found a bug still passes.

use determinism::harness::run_process_twice;
use kv_core::{Bug, Duration};
use kv_env::fake::PartitionSchedule;
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
fn the_configuration_is_one_readable_line() {
    let config = RunConfig::from_seed(7, 150);
    let line = config.to_string();
    assert!(
        line.starts_with("seed=7 nodes=3 clients=6 requests=150 quiet=30 seed="),
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
        hash, 0xc85d_a241_4993_66e8,
        "seed 1 vopr hash changed: {hash:#018x}"
    );
}

#[test]
fn the_swarm_reaches_the_replication_code() {
    let mut promotions = 0;
    let mut pulls = 0;
    let mut repairs = 0;
    let mut step_downs = 0;
    let mut forwarded = 0;
    let mut seeds_with_failover = 0;
    for seed in 0..40 {
        let report = run(&RunConfig::from_seed(seed, 100));
        let p: u64 = report.replicas.iter().map(|r| r.promotions).sum();
        promotions += p;
        seeds_with_failover += u64::from(p > 0);
        pulls += report.replicas.iter().map(|r| r.pulls_sent).sum::<u64>();
        repairs += report.replicas.iter().map(|r| r.repairs).sum::<u64>();
        step_downs += report.replicas.iter().map(|r| r.step_downs).sum::<u64>();
        forwarded += report.replicas.iter().map(|r| r.forwarded).sum::<u64>();
        assert!(report.converged_in <= Duration::from_secs(10));
    }
    println!(
        "promotions={promotions} seeds_with_failover={seeds_with_failover} pulls={pulls} repairs={repairs} step_downs={step_downs} forwarded={forwarded}"
    );
    assert!(
        seeds_with_failover >= 10,
        "{seeds_with_failover} of 40 seeds failed over"
    );
    assert!(pulls > 0, "no new primary ever had to pull a log");
    assert!(repairs > 0, "no damaged log was ever repaired");
    assert!(step_downs > 0, "no primary ever stepped down");
    assert!(forwarded > 100);
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
fn regression_seeds_still_pass() {
    // Seeds that found a bug while the module was written. Each one is a
    // run that once failed and now must not. The lesson tells their
    // stories.
    for seed in REGRESSION_SEEDS {
        run(&RunConfig::from_seed(seed, 150));
    }
}

/// 36: every disk had a latent hole, and three repairs in a row each erased
/// what came after theirs. The cluster forgot fourteen committed writes.
/// 4668: a straggling beat carried an old log length, a restarted primary
/// gathered on it and prepared a second record 25.
/// 402: a primary lost record 96 before anyone saw it, the next primary
/// numbered a new write 96, and a repair put a stray copy of the first
/// back in its place. Records carry their view since.
const REGRESSION_SEEDS: [u64; 3] = [36, 4668, 402];
