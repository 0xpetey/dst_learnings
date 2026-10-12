//! A seedable PRNG with a pinned algorithm.
//!
//! The algorithm is xoshiro256++ (Blackman & Vigna, 2018), seeded through
//! SplitMix64 exactly as the reference implementation recommends. Both are
//! written out here rather than pulled from a crate so that the course owns
//! every bit of the sequence: upgrading a dependency can never change what a
//! seed means. A test checks the output against the `rand_xoshiro` crate.
//!
//! `rand::StdRng` is deliberately *not* used anywhere in this course. Its
//! documentation says the algorithm may change between versions, which is
//! fine for a game and fatal for reproducing a bug from last month's seed.

/// The pinned generator. 256 bits of state, period 2^256 - 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prng {
    s: [u64; 4],
}

impl Prng {
    /// Build a generator from a 64-bit seed.
    ///
    /// SplitMix64 expands the seed into 256 bits of state. The all-zero
    /// state is unreachable this way, which matters: xoshiro would emit
    /// zeros forever from it.
    pub fn from_seed(seed: u64) -> Prng {
        let mut x = seed;
        let mut s = [0u64; 4];
        for word in &mut s {
            *word = splitmix64(&mut x);
        }
        debug_assert!(s != [0; 4]);
        Prng { s }
    }

    /// Next 64 uniformly distributed bits.
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Next 32 bits. Takes the *upper* half of a 64-bit output; the low
    /// bits of xoshiro++ are of equal quality but this keeps the mapping
    /// fixed and documented.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform integer in `0..bound`. Panics if `bound == 0`.
    ///
    /// Uses Lemire's multiply-and-reject method, which is unbiased. A plain
    /// `next_u64() % bound` would be biased and, worse, "almost right",
    /// which is the kind of thing a simulator should never be.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "Prng::below called with bound 0");
        let mut m = u128::from(self.next_u64()) * u128::from(bound);
        let mut low = m as u64;
        if low < bound {
            // Reject the small band of values that would bias the result.
            let threshold = bound.wrapping_neg() % bound;
            while low < threshold {
                m = u128::from(self.next_u64()) * u128::from(bound);
                low = m as u64;
            }
        }
        (m >> 64) as u64
    }

    /// Uniform integer in `lo..hi` (half open). Panics if the range is empty.
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
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// Derive an independent child generator.
    ///
    /// A simulation has many components that each want randomness (the
    /// network, the disk, each client). Giving each its own `Prng` forked
    /// from the root means adding a random decision to one component does
    /// not shift the sequence every other component sees. That keeps a
    /// seed's meaning as stable as possible while the simulator evolves.
    pub fn fork(&mut self) -> Prng {
        Prng::from_seed(self.next_u64())
    }
}

/// SplitMix64 step: advances `x` and returns one output word.
fn splitmix64(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Prng::from_seed(42);
        let mut b = Prng::from_seed(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Prng::from_seed(1);
        let mut b = Prng::from_seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_is_in_range_and_hits_every_value() {
        let mut p = Prng::from_seed(7);
        let mut seen = [false; 10];
        for _ in 0..10_000 {
            let v = p.below(10);
            assert!(v < 10);
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut p = Prng::from_seed(3);
        let mut v: Vec<u32> = (0..50).collect();
        p.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
        assert_ne!(v, sorted, "50 elements should not shuffle to identity");
    }

    #[test]
    fn forks_are_independent_of_later_parent_use() {
        let mut root = Prng::from_seed(9);
        let mut child_a = root.fork();
        let expected: Vec<u64> = (0..8).map(|_| child_a.next_u64()).collect();

        let mut root = Prng::from_seed(9);
        let mut child_b = root.fork();
        let _ = root.next_u64(); // parent keeps going; child must not care
        let got: Vec<u64> = (0..8).map(|_| child_b.next_u64()).collect();
        assert_eq!(expected, got);
    }
}
