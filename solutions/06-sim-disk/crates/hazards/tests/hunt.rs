//! Part 1 and 2 of the Module 3 task: every hazard is caught, every fix passes.

use determinism::harness::{run_process_twice, ProcessCheckError};
use determinism::{check_determinism, Prng, Trace};
use std::path::Path;

type Program = fn(&mut Prng, &mut Trace);

fn caught(name: &str, program: Program) {
    // Several runs: some hazards (select!) are coin flips per run.
    let result = check_determinism(42, 8, program);
    assert!(
        result.is_err(),
        "{name}: hazard was NOT caught by the in-process check"
    );
    eprintln!("{name}: caught\n{}", result.unwrap_err());
}

fn passes(name: &str, program: Program) {
    check_determinism(42, 8, program)
        .unwrap_or_else(|d| panic!("{name}: fix is still nondeterministic\n{d}"));
}

#[test]
fn hashmap_peer() {
    caught("hashmap_peer", hazards::hazard_hashmap_peer);
    passes("hashmap_peer", hazards::fixed_hashmap_peer);
}

#[test]
fn select_loop() {
    caught("select_loop", hazards::hazard_select_loop);
    passes("select_loop", hazards::fixed_select_loop);
}

#[test]
fn thread_rng_backoff() {
    caught("thread_rng_backoff", hazards::hazard_thread_rng_backoff);
    passes("thread_rng_backoff", hazards::fixed_thread_rng_backoff);
}

#[test]
fn static_counter() {
    caught("static_counter", hazards::hazard_static_counter);
    passes("static_counter", hazards::fixed_static_counter);
}

#[test]
fn hidden_clock() {
    caught("hidden_clock", hazards::hazard_hidden_clock);
    passes("hidden_clock", hazards::fixed_hidden_clock);
}

/// The pointer hazard is the one the in-process check usually misses. Within
/// one process the eight boxes are allocated and freed identically each run,
/// so the addresses, and the order, usually repeat. Across processes ASLR
/// moves the heap and the hashed addresses land in a different order.
#[test]
fn pointer_order_needs_the_cross_process_check() {
    let in_process = check_determinism(42, 8, hazards::hazard_pointer_order);
    eprintln!("pointer_order in-process verdict: {:?}", in_process.is_ok());
    let bin = Path::new(env!("CARGO_BIN_EXE_pointer_order"));
    match run_process_twice(bin, &[], 42) {
        Err(ProcessCheckError::Diverged(_)) => {}
        other => panic!("expected ASLR to expose the pointer sort, got {other:?}"),
    }
    run_process_twice(bin, &["--fixed"], 42).expect("sorting by id is deterministic across processes");
}
