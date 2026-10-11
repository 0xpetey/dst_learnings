//! A seedable PRNG with a pinned algorithm: xoshiro256++ seeded via SplitMix64.
//!
//! Reference: https://prng.di.unimi.it/xoshiro256plusplus.c and
//! https://prng.di.unimi.it/splitmix64.c. `tests/reference_prng.rs` checks
//! your output against the `rand_xoshiro` crate given the same state.

/// The pinned generator. 256 bits of state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prng {
    s: [u64; 4],
}

impl Prng {
    /// Expand a 64-bit seed into 256 bits of state with SplitMix64.
    pub fn from_seed(seed: u64) -> Prng {
        let _ = seed;
        todo!("Module 1: SplitMix64 the seed into four state words")
    }

    /// Next 64 uniformly distributed bits (xoshiro256++).
    pub fn next_u64(&mut self) -> u64 {
        todo!("Module 1: one xoshiro256++ step")
    }

    /// Next 32 bits: the upper half of `next_u64`.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform integer in `0..bound`, unbiased. Panics if `bound == 0`.
    pub fn below(&mut self, bound: u64) -> u64 {
        let _ = bound;
        todo!("Module 1: Lemire's method or another unbiased scheme")
    }

    /// Uniform integer in `lo..hi`. Panics if the range is empty.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        assert!(lo < hi, "Prng::range called with empty range {lo}..{hi}");
        lo + self.below(hi - lo)
    }

    /// `true` with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        assert!(denominator > 0);
        assert!(numerator <= denominator);
        self.below(denominator) < numerator
    }

    /// Pick a uniformly random element. Panics on an empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        assert!(!items.is_empty(), "Prng::pick called with empty slice");
        &items[self.below(items.len() as u64) as usize]
    }

    /// Fisher-Yates shuffle, in place.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        let _ = items;
        todo!("Module 1: Fisher-Yates using `below`")
    }

    /// Derive an independent child generator from this one.
    pub fn fork(&mut self) -> Prng {
        Prng::from_seed(self.next_u64())
    }
}
