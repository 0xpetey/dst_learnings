//! Module 11 acceptance tests, part one: the shrinker and the hunt.
//!
//! Four things must hold:
//!
//! 1. A failing seed shrinks to fewer requests and fewer faults, and
//!    the shrunk run fails the same way.
//! 2. The reproduction line is a command line: its flags parse back to
//!    the overrides that fail.
//! 3. The binary shrinks in child processes, and the hunt runs every
//!    seed and lists the failures.
//! 4. The planted Module 10 bug, a vote forgotten at a restart, is
//!    caught by seed 9916 and shrinks.

use kv_core::Bug;
use std::path::Path;
use std::process::Command;
use vopr::overrides::{reproduction, Knob, Overrides};
use vopr::shrink::{run_catching, shrink, ChildProcess, InProcess, Oracle, Outcome, ShrinkError};

fn stale_read() -> Overrides {
    Overrides {
        bugs: vec![Bug::StaleReadAfterStepDown],
        ..Overrides::default()
    }
}

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

#[test]
fn a_stale_read_shrinks_to_fewer_requests_and_fewer_faults() {
    let start = stale_read();
    let seed = 2;
    assert!(
        InProcess.run(seed, &start).is_some(),
        "seed {seed} fails to begin with"
    );
    let mut attempts = 0;
    let shrunk = shrink(seed, &start, &mut InProcess, |_| attempts += 1).unwrap();
    assert_eq!(attempts, shrunk.attempts.len());
    assert!(
        shrunk.signature.starts_with("safety: key"),
        "{}",
        shrunk.signature
    );
    let requests = shrunk.result.requests.expect("the requests were shrunk");
    assert!(requests < 150, "{requests}");
    assert!(shrunk.kept() >= 2, "{shrunk}");
    assert!(
        shrunk.result.off.contains(&Knob::Core) || shrunk.result.clients_per_node == Some(1),
        "{shrunk}"
    );
    // The shrunk configuration fails the same way, on its own.
    let again = run_catching(&shrunk.result.configure(seed)).expect("still fails");
    assert_eq!(again.signature(), shrunk.signature);
    // And the one below it passes, so the count is tight.
    let mut below = shrunk.result.clone();
    below.requests = Some(requests - 1);
    assert!(
        InProcess.run(seed, &below).is_none(),
        "{requests} was not the smallest"
    );
}

#[test]
fn the_reproduction_is_a_command_line() {
    let shrunk = shrink(2, &stale_read(), &mut InProcess, |_| {}).unwrap();
    let line = reproduction(2, &shrunk.result);
    assert!(
        line.starts_with("cargo run --release -p vopr -- --seed 2 "),
        "{line}"
    );
    assert!(line.ends_with("--bugs stale_read_after_step_down"), "{line}");
    let flags: Vec<String> = line
        .split_once(" -- ")
        .unwrap()
        .1
        .split(' ')
        .map(str::to_string)
        .collect();
    let (back, rest) = Overrides::parse(&flags).unwrap();
    assert_eq!(rest, ["--seed", "2"]);
    assert_eq!(back, shrunk.result);
    assert!(shrunk.to_string().contains(&line), "{shrunk}");
}

#[test]
fn a_passing_seed_and_an_unchanged_bug_are_reported() {
    assert_eq!(
        shrink(1, &Overrides::default(), &mut InProcess, |_| {}),
        Err(ShrinkError::Passes)
    );
    // A change that turns one failure into another is not kept, and the
    // other failure is listed.
    let shrunk = shrink(2, &stale_read(), &mut InProcess, |_| {}).unwrap();
    for a in &shrunk.attempts {
        if let Outcome::Other(sig) = &a.outcome {
            assert_ne!(sig, &shrunk.signature);
            assert!(shrunk.others.contains(sig));
        }
    }
}

#[test]
fn the_binary_shrinks_in_child_processes() {
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let mut oracle = ChildProcess::new(bin.to_path_buf());
    let f = oracle.run(2, &stale_read()).expect("the child fails");
    assert!(f.message.starts_with("safety: key"), "{}", f.message);
    assert_eq!(oracle.run(1, &Overrides::default()), None);
    assert_eq!(oracle.runs, 2);
    // The child and the library agree on the failure.
    let here = InProcess.run(2, &stale_read()).unwrap();
    assert_eq!(f.signature(), here.signature());

    let output = Command::new(bin)
        .args(["--shrink", "--seed", "2", "--bugs", "stale_read_after_step_down"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("shrunk seed 2 in "), "{stdout}");
    assert!(stdout.contains("  try --requests "), "{stdout}");
    let to = stdout
        .lines()
        .find(|l| l.starts_with("  to:   "))
        .expect("a reproduction line");
    assert!(
        to.contains("--requests ") && to.contains("--bugs stale_read_after_step_down"),
        "{to}"
    );
}

#[test]
fn the_hunt_runs_every_seed_and_lists_the_failures() {
    let bin = Path::new(env!("CARGO_BIN_EXE_vopr"));
    let output = Command::new(bin)
        .args(["--hunt", "0..8", "--bugs", "stale_read_after_step_down"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "a hunt with failures exits non-zero");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let failed = stdout.lines().filter(|l| l.contains(" FAILED ")).count();
    let ok = stdout.lines().filter(|l| l.ends_with(" ok")).count();
    assert_eq!(failed + ok, 8, "{stdout}");
    assert!(failed >= 1, "{stdout}");
    assert!(
        stdout.contains(&format!("hunt: {failed} of 8 seeds failed")),
        "{stdout}"
    );
    assert!(
        stdout.contains("cargo run --release -p vopr -- --seed 2 --bugs stale_read_after_step_down"),
        "{stdout}"
    );

    let output = Command::new(bin).args(["--hunt", "0..4"]).output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("hunt: 0 of 4 seeds failed"));
}

#[test]
fn the_forgotten_promise_is_caught_and_shrinks() {
    // Module 10's seed 562, planted again: a replica that restarts
    // without its view promise. The hunt found it on seed 9916 of
    // twenty thousand. The guard in `truncate` names it.
    let start = forgotten_promise();
    let f = InProcess.run(9916, &start).expect("seed 9916 fails");
    assert_eq!(
        f.signature(),
        "node # view #: told to truncate record #, committed through #",
        "{}",
        f.message
    );
    let shrunk = shrink(9916, &start, &mut InProcess, |_| {}).unwrap();
    let requests = shrunk.result.requests.unwrap();
    assert!(requests < 100, "{shrunk}");
    assert!(shrunk.result.bugs == vec![Bug::ForgottenPromise]);
    println!("{shrunk}");
    // Without the bug the same run passes.
    let mut fixed = shrunk.result.clone();
    fixed.bugs.clear();
    assert_eq!(InProcess.run(9916, &fixed), None);
}
