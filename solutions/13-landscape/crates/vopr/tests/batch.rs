//! Module 12 acceptance tests: the batch that CI runs.
//!
//! Five things must hold:
//!
//! 1. The seeds follow the commit. The same commit runs the same seeds,
//!    and another commit runs others.
//! 2. A failure is run twice more with its trace, and recorded with
//!    everything a bug report needs. The record reads back from its file.
//! 3. A failure that does not reproduce is reported as unstable. It is
//!    never retried into a pass.
//! 4. The cost ceiling is in units the machine does not choose, and a
//!    run over it fails the batch.
//! 5. The binary runs the batch in child processes, several at a time,
//!    writes the report, and its exit status is the verdict.

use kv_core::Bug;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use vopr::batch::{
    run_batch, seeds_for_commit, Batch, Ceiling, Children, Cost, InProcess, Observed, Progress, Record,
    Runner, Verdict,
};
use vopr::inspect::{Query, Timeline};
use vopr::overrides::{reproduction, Knob, Overrides};
use vopr::shrink::Failure;

/// The seed the hunt found for the forgotten promise, under the draws
/// of the day it was found.
fn forgotten_promise() -> Overrides {
    Overrides {
        nodes: Some(3),
        off: [Knob::Core].into_iter().collect(),
        bugs: vec![Bug::ForgottenPromise],
        ..Overrides::default()
    }
}

fn short_runs() -> Overrides {
    Overrides {
        requests: Some(40),
        ..Overrides::default()
    }
}

/// A fresh directory under the system's temporary directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vopr-batch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn the_seeds_follow_the_commit() {
    let here = seeds_for_commit("5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5", 500).unwrap();
    assert_eq!(here.end - here.start, 500);
    assert_eq!(
        here,
        seeds_for_commit("5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5", 500).unwrap(),
        "the same commit, the same seeds"
    );
    assert_eq!(here.start, 0x5b65_c2d0_a3f1, "the first twelve hex digits");
    let next = seeds_for_commit("cc16f1b7a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5", 500).unwrap();
    assert_ne!(next, here, "another commit, other seeds");
    assert!(
        next.start < 1 << 48 && here.start < 1 << 48,
        "room above every start"
    );
    // A short hash is not enough to name a batch, and a branch name is
    // not a hash.
    assert!(seeds_for_commit("5b65c2d", 500).is_err());
    assert!(seeds_for_commit("main", 500).is_err());
}

#[test]
fn a_failure_is_verified_twice_and_recorded() {
    let mut batch = Batch::new(9915..9919, forgotten_promise());
    batch.commit = Some("5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5".into());
    batch.jobs = 2;
    let mut seen = Vec::new();
    let report = run_batch(&batch, &mut InProcess, |p| {
        if let Progress::Ran(o) = p {
            seen.push(o.seed);
        }
    });
    assert_eq!(seen, [9915, 9916, 9917, 9918], "every seed, in order");
    assert_eq!((report.ran, report.passed), (4, 3));
    assert!(!report.ok());
    assert_ne!(report.sweep_hash, 0);
    assert_eq!(report.runs, 4 + 2, "the failure ran twice more");

    let [record] = report.failures.as_slice() else {
        panic!("one failure, got {:?}", report.failures);
    };
    assert_eq!(record.seed, 9916);
    assert!(record.stable(), "{record:?}");
    assert_eq!(
        record.signature,
        "node # view #: told to truncate record #, committed through #"
    );
    assert!(record.trace_hash.is_some(), "the Module 1 check, on the failure");
    assert_eq!(
        record.commit.as_deref(),
        Some("5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5")
    );
    assert_eq!(record.reproduction(), reproduction(9916, &forgotten_promise()));
    assert!(
        record
            .message
            .starts_with("node 0 view 21: told to truncate record 17"),
        "{}",
        record.message
    );

    // The record is a file, and the file is the record.
    let text = record.to_text();
    assert!(text.starts_with("seed=9916\ncommit=5b65c2d0"), "{text}");
    assert!(
        text.contains("\nreproduction=cargo run --release -p vopr -- --seed 9916 "),
        "{text}"
    );
    assert!(text.contains("\nfailure:\n  node 0 view 21"), "{text}");
    assert_eq!(Record::parse(&text).unwrap(), *record);

    // The summary names the failure and its reproduction.
    let summary = report.to_string();
    assert!(
        summary.starts_with("batch: seeds 9915..9919 ran=4 passed=3 failed=1 unstable=0 over_ceiling=0\n"),
        "{summary}"
    );
    assert!(summary.contains("\nfailed: seed=9916 trace_hash="), "{summary}");
    assert!(summary.contains(&record.reproduction()), "{summary}");
}

#[test]
fn a_shrunk_failure_carries_its_smaller_reproduction() {
    let mut batch = Batch::new(9916..9917, forgotten_promise());
    batch.shrink = true;
    let report = run_batch(&batch, &mut InProcess, |_| {});
    let record = &report.failures[0];
    let shrunk = record.shrunk.as_ref().expect("shrunk");
    assert!(shrunk.requests.unwrap() < 150, "{shrunk}");
    assert!(report.runs > 10, "the shrink's runs are counted: {}", report.runs);
    let text = record.to_text();
    assert!(
        text.contains("\nshrunk=cargo run --release -p vopr -- --seed 9916 --requests "),
        "{text}"
    );
    assert_eq!(Record::parse(&text).unwrap(), *record);
    assert!(report.to_string().contains("  shrunk: cargo run"), "{report}");
}

/// A pretend runner. Seed 5 fails once and then passes. Seed 6 fails
/// the same way every time, with a different trace every time. Every
/// other seed passes at a fixed cost.
#[derive(Default)]
struct Flaky {
    calls: BTreeMap<u64, u64>,
}

impl Runner for Flaky {
    fn run_many(&mut self, seeds: &[u64], _: &Overrides, trace: bool) -> Vec<Observed> {
        seeds
            .iter()
            .map(|&seed| {
                let call = self.calls.entry(seed).or_default();
                *call += 1;
                let failure = Failure {
                    message: format!("safety: lost record {seed}"),
                };
                let (verdict, output_hash) = match seed {
                    5 if *call == 1 => (Verdict::Failed(failure), 1),
                    6 => (Verdict::Failed(failure), *call),
                    _ => (
                        Verdict::Passed {
                            hash: seed,
                            cost: Cost {
                                events: 1_000 * seed,
                                requests: 10,
                                check_steps: 7,
                            },
                        },
                        seed,
                    ),
                };
                Observed {
                    seed,
                    verdict,
                    output_hash,
                    output: trace.then(|| format!("trace of {seed} run {call}")),
                    wall: Duration::ZERO,
                }
            })
            .collect()
    }
}

#[test]
fn an_unstable_failure_is_reported_and_never_retried_into_a_pass() {
    let batch = Batch::new(4..8, Overrides::default());
    let report = run_batch(&batch, &mut Flaky::default(), |_| {});
    assert_eq!((report.ran, report.passed), (4, 2));
    assert!(!report.ok());
    assert_eq!(report.failures.len(), 2);
    assert_eq!(report.unstable().count(), 2);

    let five = &report.failures[0];
    assert_eq!(five.seed, 5);
    assert_eq!(
        five.disagreement.as_deref(),
        Some("pass"),
        "failed, then passed: that is a finding"
    );
    assert_eq!(five.trace_hash, None);
    let six = &report.failures[1];
    assert_eq!(six.seed, 6);
    let why = six.disagreement.as_deref().unwrap();
    assert!(why.starts_with("the same failure with another trace"), "{why}");

    let summary = report.to_string();
    assert!(summary.contains(" failed=2 unstable=2 "), "{summary}");
    assert!(
        summary.contains("unstable: seed=5 first=\"safety: lost record #\" then=\"pass\""),
        "{summary}"
    );
    for r in &report.failures {
        let text = r.to_text();
        assert!(text.contains("\ntrace_hash=none\n"), "{text}");
        assert_eq!(Record::parse(&text).unwrap(), *r);
    }
}

#[test]
fn the_ceiling_is_in_the_seeds_units() {
    let mut batch = Batch::new(0..4, short_runs());
    batch.jobs = 4;
    let first = run_batch(&batch, &mut InProcess, |_| {});
    assert!(first.ok(), "{first}");
    assert_eq!(first.passed, 4);
    let (seed, cost) = first.max_events.unwrap();
    assert!(
        cost.events_per_request() > 0 && cost.events_per_request() < 2_000,
        "{cost:?}"
    );
    assert!(seed < 4);
    // The cost is a property of the seed, so a second batch sees it again.
    let again = run_batch(&batch, &mut InProcess, |_| {});
    assert_eq!(again.max_events, first.max_events);
    assert_eq!(again.max_steps, first.max_steps);
    assert_eq!(again.sweep_hash, first.sweep_hash);

    // A ceiling below the cost is a failed batch, with each seed named.
    batch.ceiling = Ceiling {
        events_per_request: 1,
        check_steps: 1,
    };
    let mut over = Vec::new();
    let report = run_batch(&batch, &mut InProcess, |p| {
        if let Progress::OverCeiling(seed, _) = p {
            over.push(seed);
        }
    });
    assert_eq!(over, [0, 1, 2, 3]);
    assert_eq!(report.over_ceiling.len(), 4);
    assert!(!report.ok());
    assert!(
        report.over_ceiling[0].1.starts_with("events per request "),
        "{report}"
    );
    assert!(
        report
            .to_string()
            .contains("over ceiling: seed=0 events per request"),
        "{report}"
    );

    // A pretend runner at a fixed cost is over the default ceiling too.
    let report = run_batch(
        &Batch::new(100..102, Overrides::default()),
        &mut Flaky::default(),
        |_| {},
    );
    assert_eq!(report.over_ceiling.len(), 2, "{report}");
}

#[test]
fn a_budget_stops_the_batch_between_chunks() {
    let mut batch = Batch::new(0..64, short_runs());
    batch.jobs = 2;
    batch.budget = Some(Duration::ZERO);
    let mut stopped = None;
    let report = run_batch(&batch, &mut InProcess, |p| {
        if let Progress::Stopped(left) = p {
            stopped = Some(left);
        }
    });
    assert_eq!(report.ran, 0, "nothing starts once the budget is spent");
    assert_eq!(stopped, Some(64));
    assert!(report.ok(), "a batch cut short is not a failed batch");
}

#[test]
fn children_agree_with_the_library_and_come_back_in_order() {
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let mut children = Children::new(bin.to_path_buf(), 3);
    let seeds = [3, 1, 2, 0];
    let observed = children.run_many(&seeds, &short_runs(), false);
    assert_eq!(children.runs, 4);
    let here = InProcess.run_many(&seeds, &short_runs(), false);
    for (child, lib) in observed.iter().zip(&here) {
        assert_eq!(child.seed, lib.seed);
        let (Verdict::Passed { hash, cost }, Verdict::Passed { hash: h, cost: c }) =
            (&child.verdict, &lib.verdict)
        else {
            panic!("{:?} {:?}", child.verdict, lib.verdict);
        };
        assert_eq!(hash, h, "seed {}", child.seed);
        assert_eq!(cost, c, "seed {}", child.seed);
        assert_eq!(child.output, None);
    }
    // With the trace, the output is the trace, and it hashes the same twice.
    let a = children.run_many(&[9916], &forgotten_promise(), true).remove(0);
    let b = children.run_many(&[9916], &forgotten_promise(), true).remove(0);
    assert!(matches!(a.verdict, Verdict::Failed(_)));
    assert_eq!(a.output_hash, b.output_hash);
    let trace = a.output.expect("the trace");
    assert!(trace.lines().count() > 1000, "{} lines", trace.lines().count());
    assert_eq!(trace, b.output.unwrap());
}

#[test]
fn the_binary_runs_a_batch_and_writes_the_report() {
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let dir = scratch("report");
    let output = Command::new(bin)
        .args([
            "--batch",
            "4",
            "--from",
            "9915",
            "--jobs",
            "2",
            "--nodes",
            "3",
            "--no-core",
        ])
        .args([
            "--bugs",
            "forgotten_promise",
            "--commit",
            "5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5",
        ])
        .arg("--report")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success(), "a batch with a failure exits non-zero");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("seed=9915 ok events/request="), "{stdout}");
    assert!(stdout.contains("seed=9916 FAILED node # view #"), "{stdout}");
    assert!(
        stdout.contains("seed=9916 verified twice, trace_hash="),
        "{stdout}"
    );
    assert!(
        stdout.contains("batch: seeds 9915..9919 ran=4 passed=3 failed=1 unstable=0 over_ceiling=0"),
        "{stdout}"
    );
    assert!(stdout.contains("report: 3 files in "), "{stdout}");

    let summary = std::fs::read_to_string(dir.join("summary.txt")).unwrap();
    assert!(
        summary.contains("commit=5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5\n"),
        "{summary}"
    );
    assert!(summary.contains("\nsweep_hash="), "{summary}");
    let record = Record::parse(&std::fs::read_to_string(dir.join("seed-9916.txt")).unwrap()).unwrap();
    assert_eq!(record.seed, 9916);
    assert!(record.stable());
    assert_eq!(record.reproduction(), reproduction(9916, &forgotten_promise()));

    // The trace file is the failing run, and the inspector reads it.
    let trace = std::fs::read_to_string(dir.join("seed-9916.trace")).unwrap();
    assert!(
        trace.starts_with("seed=9916 nodes=3 "),
        "{}",
        trace.lines().next().unwrap()
    );
    let timeline = Timeline::parse(trace.lines());
    let rendered = timeline.render(&Query::new("seed-9916.trace"));
    assert!(rendered.contains("n0 gathering view=21"), "{rendered}");
    let _ = std::fs::remove_dir_all(&dir);

    // A clean batch exits zero and ends in the sweep hash.
    let output = Command::new(bin)
        .args(["--batch", "3", "--from", "0", "--jobs", "3", "--requests", "40"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("batch: seeds 0..3 ran=3 passed=3 failed=0"),
        "{stdout}"
    );
    assert!(stdout.contains("\nsweep_hash="), "{stdout}");

    // Seeds from the commit, and a batch that names neither is a usage error.
    let output = Command::new(bin)
        .args([
            "--batch",
            "1",
            "--commit",
            "5b65c2d0a3f1e8b7c6d5e4f3a2b1c0d9e8f7a6b5",
            "--requests",
            "20",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("batch: seeds 100492618277873..100492618277874 ")
    );
    let output = Command::new(bin).args(["--batch", "1"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}
