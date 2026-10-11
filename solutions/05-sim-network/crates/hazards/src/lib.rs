//! The hazard hunt.
//!
//! Each `hazard_*` function is idiomatic Rust that looks deterministic and is
//! not. Each `fixed_*` function is the same logic with the recommended
//! replacement. The tests in `tests/hunt.rs` run both through the Module 1
//! determinism check and assert that the hazard is caught and the fix passes.
//!
//! This crate is the *only* one in the workspace allowed to use the banned
//! APIs, so the lints are disabled at the crate root. In `kv-core` the same
//! code would not compile (`no_std`) or would fail `cargo clippy`.

#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use determinism::{trace, Prng, Trace};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// 1. HashMap iteration order: picking "the first healthy peer".

pub fn hazard_hashmap_peer(_prng: &mut Prng, t: &mut Trace) {
    let peers: HashMap<u32, bool> = (0..32).map(|id| (id, id % 3 != 0)).collect();
    let chosen = peers.iter().find(|(_, &healthy)| healthy).map(|(&id, _)| id);
    trace!(t, "chose peer {chosen:?}");
}

pub fn fixed_hashmap_peer(_prng: &mut Prng, t: &mut Trace) {
    let peers: BTreeMap<u32, bool> = (0..32).map(|id| (id, id % 3 != 0)).collect();
    let chosen = peers.iter().find(|(_, &healthy)| healthy).map(|(&id, _)| id);
    trace!(t, "chose peer {chosen:?}");
}

// ---------------------------------------------------------------------------
// 2. tokio::select! picks a ready branch at random by design.

pub fn hazard_select_loop(_prng: &mut Prng, t: &mut Trace) {
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        let (tx_a, mut rx_a) = tokio::sync::mpsc::channel::<u32>(64);
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel::<u32>(64);
        for i in 0..32 {
            tx_a.send(i).await.unwrap();
            tx_b.send(100 + i).await.unwrap();
        }
        // Both channels are always ready, so each iteration is a coin flip.
        for _ in 0..64 {
            let got = tokio::select! {
                Some(v) = rx_a.recv() => v,
                Some(v) = rx_b.recv() => v,
            };
            trace!(t, "handled {got}");
        }
    });
}

pub fn fixed_select_loop(_prng: &mut Prng, t: &mut Trace) {
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        let (tx_a, mut rx_a) = tokio::sync::mpsc::channel::<u32>(64);
        let (tx_b, mut rx_b) = tokio::sync::mpsc::channel::<u32>(64);
        for i in 0..32 {
            tx_a.send(i).await.unwrap();
            tx_b.send(100 + i).await.unwrap();
        }
        for _ in 0..64 {
            // `biased;` polls branches top to bottom. The *order* is now a
            // design decision you can read, instead of a dice roll.
            let got = tokio::select! {
                biased;
                Some(v) = rx_a.recv() => v,
                Some(v) = rx_b.recv() => v,
            };
            trace!(t, "handled {got}");
        }
    });
}

// ---------------------------------------------------------------------------
// 3. Backoff jitter from the OS random number generator.

pub fn hazard_thread_rng_backoff(_prng: &mut Prng, t: &mut Trace) {
    let mut delay_ms = 10u64;
    for attempt in 0..5 {
        let jitter = rand::random::<u64>() % delay_ms;
        trace!(t, "attempt {attempt} sleeps {}ms", delay_ms + jitter);
        delay_ms *= 2;
    }
}

pub fn fixed_thread_rng_backoff(prng: &mut Prng, t: &mut Trace) {
    let mut delay_ms = 10u64;
    for attempt in 0..5 {
        let jitter = prng.below(delay_ms);
        trace!(t, "attempt {attempt} sleeps {}ms", delay_ms + jitter);
        delay_ms *= 2;
    }
}

// ---------------------------------------------------------------------------
// 4. Keying on a pointer. Deterministic within a process (the allocator hands
//    the second run the same addresses), different across processes (ASLR).
//    See the `pointer_order` binary and the cross-process test.
//
//    Note what does *not* expose this: sorting by raw address. ASLR slides
//    the whole heap, so eight boxes allocated in a row keep their relative
//    order. The moment the address is *hashed* (a map keyed by pointer, a
//    pointer mixed into an id) the slide changes everything.

pub struct Replica {
    pub id: u32,
}

/// A stable hash. The hasher is deterministic; the input is not.
fn fnv(x: u64) -> u64 {
    x.to_le_bytes().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

pub fn hazard_pointer_order(_prng: &mut Prng, t: &mut Trace) {
    let replicas: Vec<Box<Replica>> = (0..8).map(|id| Box::new(Replica { id })).collect();
    // "Keyed by identity, with a fixed hasher, so it must be deterministic."
    let by_identity: BTreeMap<u64, &Replica> = replicas
        .iter()
        .map(|b| (fnv(&**b as *const Replica as u64), &**b))
        .collect();
    let order: Vec<u32> = by_identity.values().map(|r| r.id).collect();
    trace!(t, "leader election order {order:?}");
}

pub fn fixed_pointer_order(_prng: &mut Prng, t: &mut Trace) {
    let replicas: Vec<Box<Replica>> = (0..8).map(|id| Box::new(Replica { id })).collect();
    // Identity is the replica id, which the program chose, not the allocator.
    let by_identity: BTreeMap<u64, &Replica> =
        replicas.iter().map(|b| (fnv(u64::from(b.id)), &**b)).collect();
    let order: Vec<u32> = by_identity.values().map(|r| r.id).collect();
    trace!(t, "leader election order {order:?}");
}

// ---------------------------------------------------------------------------
// 5. A static shared by two "nodes" in one process.

static REQUESTS_SERVED: AtomicU64 = AtomicU64::new(0);

pub fn hazard_static_counter(prng: &mut Prng, t: &mut Trace) {
    // Two simulated nodes, each believing it owns the counter.
    for node in 0..2 {
        for _ in 0..prng.range(1, 4) {
            REQUESTS_SERVED.fetch_add(1, Ordering::Relaxed);
        }
        trace!(
            t,
            "node {node} has served {}",
            REQUESTS_SERVED.load(Ordering::Relaxed)
        );
    }
}

#[derive(Default)]
pub struct Node {
    requests_served: u64,
}

pub fn fixed_static_counter(prng: &mut Prng, t: &mut Trace) {
    let mut nodes = [Node::default(), Node::default()];
    for (i, node) in nodes.iter_mut().enumerate() {
        for _ in 0..prng.range(1, 4) {
            node.requests_served += 1;
        }
        trace!(t, "node {i} has served {}", node.requests_served);
    }
}

// ---------------------------------------------------------------------------
// 6. A "dependency" that reads the clock internally. Pretend `sneaky` is a
//    crate from crates.io whose README says nothing about time.

pub mod sneaky {
    /// Decides whether a cached entry is fresh. Looks pure. Is not.
    pub fn is_fresh(inserted_at_ms: u64, ttl_ms: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        now.saturating_sub(inserted_at_ms) < ttl_ms
    }

    /// The fix: the caller supplies the time. Same logic, no hidden input.
    pub fn is_fresh_at(now_ms: u64, inserted_at_ms: u64, ttl_ms: u64) -> bool {
        now_ms.saturating_sub(inserted_at_ms) < ttl_ms
    }
}

pub fn hazard_hidden_clock(_prng: &mut Prng, t: &mut Trace) {
    fn wall_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
    let inserted_at_ms = wall_ms();
    // Spin so that a millisecond or two passes; a real program would have
    // done real work here.
    let mut x = 0u64;
    while wall_ms() < inserted_at_ms + 2 {
        x = x.wrapping_add(1);
    }
    std::hint::black_box(x);
    trace!(
        t,
        "inserted at {inserted_at_ms} fresh: {}",
        sneaky::is_fresh(inserted_at_ms, 1)
    );
}

pub fn fixed_hidden_clock(prng: &mut Prng, t: &mut Trace) {
    let inserted_at_ms = 1_000;
    let now_ms = inserted_at_ms + prng.below(5);
    trace!(
        t,
        "fresh after {}ms: {}",
        now_ms - inserted_at_ms,
        sneaky::is_fresh_at(now_ms, inserted_at_ms, 1)
    );
}
