//! Module 11 acceptance tests, part two: the trace inspector.
//!
//! Three things must hold:
//!
//! 1. The inspector's picture of a replica agrees with the replica. A
//!    world is run, its trace is parsed, and the snapshot at the end
//!    says what the replicas say.
//! 2. A failing seed's trace, printed up to the assertion, is readable:
//!    the snapshot at the end shows the split the guard refused.
//! 3. The binary answers `--inspect` with the snapshot and the
//!    selected lines.

use checker::{Workload, WorkloadConfig};
use determinism::Prng;
use kv_core::{client, Duration};
use kv_env::fake::{World, WorldConfig};
use std::path::Path;
use std::process::Command;
use vopr::inspect::{parse_time, Query, Timeline};

fn world(seed: u64) -> World {
    let mut prng = Prng::from_seed(seed);
    let mut config = WorldConfig::swarm(&mut prng);
    config.keep_trace_lines = true;
    let mut w = World::cluster(3, config);
    for id in 1..=6 {
        w.add_client(
            id,
            client::Config {
                initial_timeout: Duration::from_millis(100),
                max_timeout: Duration::from_millis(1000),
                max_attempts: 50,
            },
            (id % 3) as u8,
        );
    }
    w
}

#[test]
fn the_snapshot_agrees_with_the_replicas() {
    let mut checked = 0;
    for seed in 0..6 {
        let mut w = world(seed);
        let mut workload = Workload::new(WorkloadConfig::default(), Prng::from_seed(seed));
        w.run_workload(&mut workload, 60);
        let trace = w.trace();
        let timeline = Timeline::parse(trace.lines().iter().map(String::as_str));
        assert_eq!(timeline.nodes, 3);
        assert_eq!(timeline.events.len(), trace.lines().len(), "every line parses");
        let snapshot = timeline.snapshot(timeline.end());
        assert_eq!(snapshot.lines, trace.lines().len());
        for n in 0..3u8 {
            let seen = &snapshot.replicas[usize::from(n)];
            assert_eq!(seen.up, w.is_up(n), "seed {seed} n{n}");
            if !w.is_up(n) {
                continue;
            }
            assert_eq!(seen.paused, w.is_paused(n), "seed {seed} n{n}");
            let r = w.replica(n);
            assert_eq!(seen.status, r.status().to_string(), "seed {seed} n{n}");
            assert_eq!(seen.view, r.view(), "seed {seed} n{n}");
            // The log is what the last beat said. The replica may have
            // appended since, never dropped below its commit.
            assert!(
                seen.top <= r.len() || seen.view < r.view(),
                "seed {seed} n{n}: {seen} vs len={}",
                r.len()
            );
            assert!(
                seen.commit <= r.commit() || seen.view < r.view(),
                "seed {seed} n{n}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 12);
}

#[test]
fn a_failing_trace_shows_the_split() {
    // Seed 9916 under the forgotten promise, shrunk. The trace stops at
    // the assertion, and the last snapshot shows two logs: seventeen
    // records committed under normal view 18, and sixteen under 19.
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let output = Command::new(bin)
        .args([
            "--seed",
            "9916",
            "--requests",
            "57",
            "--nodes",
            "3",
            "--no-core",
            "--bugs",
            "forgotten_promise",
            "--trace",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("told to truncate record 17, committed through 17"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let timeline = Timeline::parse(stdout.lines());
    assert!(timeline.events.len() > 1000);
    let end = timeline.snapshot(timeline.end());
    let views: Vec<(u64, u64, u64)> = end
        .replicas
        .iter()
        .map(|r| (r.log_view, r.top, r.commit))
        .collect();
    assert!(views.contains(&(18, 17, 17)), "{end}");
    assert!(views.contains(&(19, 16, 16)), "{end}");
    assert!(end.replicas.iter().all(|r| r.view == 21), "{end}");
    // The replica that forgot: the primary of view 19 crashed, and its
    // first status after the restart names a lower view. It rejoined
    // view 18 and helped its old primary commit record 17 there.
    let primary_of_19 = timeline
        .events
        .iter()
        .find(|e| e.text.ends_with("status view=19 primary"))
        .and_then(|e| e.node)
        .expect("view 19 had a primary");
    let mut q = Query::new("-");
    q.replica = Some(primary_of_19);
    let lines = timeline.select(&q);
    let restart = lines
        .iter()
        .position(|e| e.text == format!("n{primary_of_19} restart"))
        .expect("the primary of 19 restarted");
    let after = lines[restart..]
        .iter()
        .find_map(|e| e.text.strip_prefix(&format!("n{primary_of_19} status view=")))
        .expect("a status after the restart");
    let view: u64 = after.split(' ').next().unwrap().parse().unwrap();
    assert!(view < 19, "n{primary_of_19} came back in view {view}");
    // Every line of view 19 is either about that view or from a replica in it.
    q.replica = None;
    q.view = Some(19);
    let in_view = timeline.select(&q);
    assert!(!in_view.is_empty());
    assert!(in_view.iter().all(|e| e.view == Some(19)));
}

#[test]
fn the_binary_inspects_a_trace_file() {
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let dir = std::env::temp_dir().join(format!("vopr-inspect-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("seed3.trace");
    let output = Command::new(bin)
        .args(["--seed", "3", "--requests", "40", "--trace"])
        .output()
        .unwrap();
    assert!(output.status.success());
    std::fs::write(&file, &output.stdout).unwrap();
    let file = file.to_str().unwrap();

    let at = "--at";
    let output = Command::new(bin)
        .args(["--inspect", file, at, "250ms"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("at 0.25s after "), "{stdout}");
    assert!(stdout.contains("  n0 ") && stdout.contains("  n2 "), "{stdout}");
    assert!(
        stdout.lines().filter(|l| l.starts_with("0.")).count() <= 12,
        "{stdout}"
    );

    let output = Command::new(bin)
        .args([
            "--inspect",
            file,
            "--replica",
            "1",
            "--since",
            "100ms",
            "--until",
            "200ms",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let selected: Vec<&str> = stdout.lines().filter(|l| l.starts_with("0.1")).collect();
    assert!(!selected.is_empty(), "{stdout}");
    // Its own lines, lines that name it, and its timer.
    assert!(
        selected.iter().all(|l| l.contains("n1") || l.contains("node=1")),
        "{stdout}"
    );
    assert!(stdout.contains(" lines selected"), "{stdout}");
    assert!(stdout.contains("at 0.2s after "), "{stdout}");

    assert_eq!(parse_time("0.25s"), Some(250_000_000));
    let _ = std::fs::remove_dir_all(&dir);
}
