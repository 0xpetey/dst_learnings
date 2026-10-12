//! Proves the hand-rolled xoshiro256++ in `determinism::Prng` is the real
//! algorithm, by comparing against the `rand_xoshiro` crate given identical
//! 256-bit state. If this test fails, our seeds would not be reproducible by
//! anyone using the reference implementation.

use determinism::Prng;
use rand_core::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;

/// Re-derive the SplitMix64 expansion so we can hand the same state to the
/// reference crate. (Kept in the test so the library stays dependency free.)
fn splitmix_state(seed: u64) -> [u8; 32] {
    let mut x = seed;
    let mut out = [0u8; 32];
    for chunk in out.chunks_exact_mut(8) {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes());
    }
    out
}

#[test]
fn matches_rand_xoshiro_for_many_seeds() {
    for seed in [0u64, 1, 2, 42, 0xDEAD_BEEF, u64::MAX] {
        let mut ours = Prng::from_seed(seed);
        let mut reference = Xoshiro256PlusPlus::from_seed(splitmix_state(seed));
        for i in 0..10_000 {
            assert_eq!(ours.next_u64(), reference.next_u64(), "seed {seed} output {i}");
        }
    }
}

#[test]
fn known_answer_seed_zero() {
    // The first outputs for seed 0. Pinned so a refactor of `Prng` cannot
    // silently change what every recorded seed in the course means.
    let mut p = Prng::from_seed(0);
    let got: Vec<u64> = (0..4).map(|_| p.next_u64()).collect();
    assert_eq!(
        got,
        [
            0x53175d61490b23df,
            0x61da6f3dc380d507,
            0x5c0fdf91ec9a7bfc,
            0x02eebf8c3bbe5e1a
        ]
    );
}
