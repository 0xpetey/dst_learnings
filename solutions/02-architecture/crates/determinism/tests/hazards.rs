//! The nondeterminism catalogue, as tests.
//!
//! Each test introduces exactly one source of nondeterminism into an
//! otherwise pure program and asserts that the determinism check catches it.
//! Together they are the "before" picture: this is what the check must guard
//! against for the rest of the course. See `tests/cross_process.rs` for the
//! hazards that an in-process check *cannot* see.

use determinism::{check_determinism, run_twice, trace, Trace};
use std::collections::HashMap;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A control: a program that only consumes the seed is deterministic.
#[test]
fn control_pure_program_is_deterministic() {
    let hash = check_determinism(2024, 10, |prng, t| {
        let mut items: Vec<u32> = (0..16).collect();
        prng.shuffle(&mut items);
        trace!(t, "order {items:?}");
        trace!(t, "coin {}", prng.chance(1, 2));
    })
    .expect("pure function of seed");
    // The hash of a given seed never changes between test runs either.
    assert_eq!(
        hash,
        check_determinism(2024, 2, |prng, t| {
            let mut items: Vec<u32> = (0..16).collect();
            prng.shuffle(&mut items);
            trace!(t, "order {items:?}");
            trace!(t, "coin {}", prng.chance(1, 2));
        })
        .unwrap()
    );
}

/// Hazard 1: the wall clock.
#[test]
fn wall_clock_is_caught() {
    let err = run_twice(1, |_prng, t| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        trace!(t, "started at {}ns", now.as_nanos());
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// Hazard 2: the monotonic clock. Measuring *how long* something took is
/// just as nondeterministic as reading the date.
#[test]
fn monotonic_clock_is_caught() {
    let err = run_twice(1, |_prng, t| {
        let start = Instant::now();
        let mut x = 0u64;
        for i in 0..200_000u64 {
            x = x.wrapping_mul(31).wrapping_add(i);
        }
        trace!(t, "work took {}ns (x={x})", start.elapsed().as_nanos());
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// Hazard 3: OS thread scheduling. Several threads append to a shared
/// vector; the interleaving is the scheduler's choice. A barrier makes sure
/// the threads really overlap instead of finishing one after another.
#[test]
fn thread_interleaving_is_caught() {
    let err = check_determinism(1, 4, |_prng, t| {
        let log = Arc::new(Mutex::new(Vec::<u8>::with_capacity(40_000)));
        let barrier = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4u8)
            .map(|id| {
                let log = Arc::clone(&log);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..10_000 {
                        log.lock().unwrap().push(id);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let log = log.lock().unwrap();
        // Record the interleaving, not just the totals: totals are the same
        // every run (each thread pushes 10_000 times) and would hide the bug.
        let mut h = Trace::hash_only();
        trace!(h, "{:?}", &log[..]);
        trace!(t, "interleaving {:#018x}", h.hash());
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// Hazard 4: I/O completion order. Modelled with worker threads "completing"
/// in whatever order the OS runs them and reporting through a channel.
#[test]
fn completion_order_is_caught() {
    // Eight runs, not two: under load the scheduler can hand two runs the
    // same order, and a hazard test that fails one time in forty is a
    // hazard of its own.
    let err = check_determinism(1, 8, |_prng, t| {
        let (tx, rx) = std::sync::mpsc::channel::<u8>();
        let barrier = Arc::new(Barrier::new(8));
        for id in 0..8u8 {
            let tx = tx.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                // Tiny, scheduler-dependent amount of work before completing.
                let mut x = u64::from(id);
                for i in 0..(1000 * u64::from(id + 1)) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(i);
                }
                std::hint::black_box(x);
                tx.send(id).unwrap();
            });
        }
        drop(tx);
        let order: Vec<u8> = rx.iter().collect();
        trace!(t, "completions {order:?}");
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// Hazard 5: an OS-seeded random number generator. `rand::random` is the
/// idiomatic thing to reach for and exactly the wrong one here.
#[test]
fn os_random_is_caught() {
    let err = run_twice(1, |_prng, t| {
        let jitter: u64 = rand::random::<u64>();
        trace!(t, "backoff jitter {jitter}ms");
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// The fix for hazard 5: all randomness flows from the seed.
#[test]
fn seeded_random_is_fine() {
    check_determinism(1, 10, |prng, t| {
        let jitter = prng.below(1000);
        trace!(t, "backoff jitter {jitter}ms");
    })
    .expect("seeded jitter is deterministic");
}

/// Hazard 6: `HashMap` iteration order. Every `HashMap::new()` gets a fresh
/// random hasher, so two maps with identical contents iterate differently,
/// even inside one process.
#[test]
fn hashmap_iteration_order_is_caught() {
    let err = run_twice(1, |_prng, t| {
        let peers: HashMap<u32, &str> = (0..64).map(|i| (i, "peer")).collect();
        let order: Vec<u32> = peers.keys().copied().collect();
        trace!(t, "peer order {order:?}");
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// The fix for hazard 6: an ordered map. (A `HashMap` with a fixed
/// `BuildHasher` also works; Module 3 bans the default one by lint.)
#[test]
fn btreemap_iteration_order_is_fine() {
    check_determinism(1, 10, |_prng, t| {
        let peers: std::collections::BTreeMap<u32, &str> = (0..64).map(|i| (i, "peer")).collect();
        let order: Vec<u32> = peers.keys().copied().collect();
        trace!(t, "peer order {order:?}");
    })
    .expect("ordered map iterates deterministically");
}

/// Hazard 7: memory addresses. NOTE: this test documents that the in-process
/// check is *not* reliable here. The allocator often hands the second run the
/// same address the first run freed, so the check may pass. ASLR makes the
/// address differ between processes; see `tests/cross_process.rs`.
#[test]
fn memory_address_may_escape_in_process_check() {
    let result = run_twice(1, |_prng, t| {
        let b = Box::new(0xABu8);
        trace!(t, "node lives at {:p}", &*b);
    });
    // Either outcome is "allowed" here; the point is that passing proves
    // nothing. The cross-process test is the real one.
    eprintln!(
        "in-process address check: {:?}",
        result.as_ref().map(|_| "passed (address reused)")
    );
}

/// Hazard 8: hidden state outside the seed. A `static` that survives from
/// one run to the next is the single-process version of two simulated nodes
/// sharing a global, and the check catches it on the second run.
#[test]
fn static_state_is_caught() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static REQUESTS_SERVED: AtomicU64 = AtomicU64::new(0);
    let err = run_twice(1, |prng, t| {
        for _ in 0..prng.range(1, 5) {
            REQUESTS_SERVED.fetch_add(1, Ordering::Relaxed);
        }
        trace!(t, "served so far {}", REQUESTS_SERVED.load(Ordering::Relaxed));
    })
    .unwrap_err();
    assert_eq!(err.index, Some(0));
}

/// The fix for hazard 8: state hangs off a struct created inside the run.
#[test]
fn per_run_state_is_fine() {
    struct Node {
        requests_served: u64,
    }
    check_determinism(1, 10, |prng, t| {
        let mut node = Node { requests_served: 0 };
        for _ in 0..prng.range(1, 5) {
            node.requests_served += 1;
        }
        trace!(t, "served so far {}", node.requests_served);
    })
    .expect("per-run state is deterministic");
}

/// Hazard 9: floating point is deterministic on one machine and toolchain,
/// so the check passes here. It is listed because `f64::sin`, `exp`, `powf`
/// and friends come from the platform's libm and can differ *across*
/// machines, which no local check can see. Keep transcendentals out of core
/// logic or use a pure-Rust libm.
#[test]
fn floating_point_passes_locally_but_is_not_portable() {
    check_determinism(1, 10, |prng, t| {
        let x = prng.below(1000) as f64 / 7.0;
        trace!(t, "sin {} exp {}", x.sin().to_bits(), x.exp().to_bits());
    })
    .expect("same machine, same libm");
}
