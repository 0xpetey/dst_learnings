//! Hazards the in-process check cannot see.
//!
//! Memory addresses (ASLR) and process-wide inputs like environment variables
//! are fixed for the life of a process, so running a closure twice in one
//! process sees the same values twice. Spawning the program twice does not.

use determinism::harness::{run_process_twice, ProcessCheckError};
use std::path::PathBuf;

fn example_binary() -> PathBuf {
    // Cargo builds bin targets before integration tests and exposes their paths.
    PathBuf::from(env!("CARGO_BIN_EXE_trace_hash"))
}

#[test]
fn pure_program_matches_across_processes() {
    let hash = run_process_twice(&example_binary(), &[], 7).expect("pure program");
    let again = run_process_twice(&example_binary(), &[], 7).unwrap();
    assert_eq!(hash, again);
}

#[test]
fn heap_addresses_differ_across_processes() {
    match run_process_twice(&example_binary(), &["--hazard", "address"], 7) {
        Err(ProcessCheckError::Diverged(d)) => assert_ne!(d.first_hash, d.second_hash),
        other => panic!("expected ASLR to change the trace, got {other:?}"),
    }
}

#[test]
fn environment_is_an_uncontrolled_input() {
    // Same environment both times: passes, which proves nothing about the
    // program being a pure function of its seed.
    run_process_twice(&example_binary(), &["--hazard", "env"], 7).expect("same env, same hash");
    // The only way to see this hazard is to vary the input and notice the
    // output moved. Module 3 makes `std::env` unreachable from core instead.
    let bin = example_binary();
    let run = |host: &str| {
        let out = std::process::Command::new(&bin)
            .args(["--hazard", "env", "--seed", "7"])
            .env("DST_HOSTNAME", host)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert_ne!(run("alpha"), run("beta"));
}
