//! A probability, as a fraction, rolled against a [`Prng`].
//!
//! Every fault in the simulator is a `Chance`: a message is lost with
//! probability `loss`, a sector rots with probability `corrupt_write`, a
//! fault point fires with probability `fire`. The type lives here, next to
//! the generator, because a probability is only meaningful together with
//! the dice that roll it. Module 5 introduced it in the network model;
//! Module 7 moved it here so that the core's fault points can use it
//! without depending on the simulator.
//!
//! The fraction is kept as two integers, never a float. `25/100` and
//! `1/4` roll the same way on every platform, and the trace can print
//! them exactly.

use crate::Prng;
use core::fmt;

/// A probability, as `numerator / denominator`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chance {
    num: u32,
    den: u32,
}

impl Chance {
    pub const NEVER: Chance = Chance { num: 0, den: 1 };
    pub const ALWAYS: Chance = Chance { num: 1, den: 1 };

    pub const fn new(num: u32, den: u32) -> Chance {
        assert!(den > 0, "Chance with a zero denominator");
        assert!(num <= den, "Chance above one");
        Chance { num, den }
    }

    pub const fn percent(p: u32) -> Chance {
        Chance::new(p, 100)
    }

    pub const fn per_mille(p: u32) -> Chance {
        Chance::new(p, 1000)
    }

    pub const fn is_never(self) -> bool {
        self.num == 0
    }

    pub const fn is_always(self) -> bool {
        self.num == self.den
    }

    pub const fn numerator(self) -> u32 {
        self.num
    }

    pub const fn denominator(self) -> u32 {
        self.den
    }

    /// Roll the dice. Spends no randomness on `NEVER` or `ALWAYS`, so that
    /// turning a fault off does not shift every other draw.
    pub fn roll(self, prng: &mut Prng) -> bool {
        if self.is_never() {
            return false;
        }
        if self.is_always() {
            return true;
        }
        prng.chance(u64::from(self.num), u64::from(self.den))
    }
}

impl fmt::Display for Chance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_and_always_spend_no_randomness() {
        let mut a = Prng::from_seed(1);
        let mut b = Prng::from_seed(1);
        assert!(!Chance::NEVER.roll(&mut a));
        assert!(Chance::ALWAYS.roll(&mut a));
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn a_quarter_fires_about_a_quarter_of_the_time() {
        let mut p = Prng::from_seed(2);
        let hits = (0..40_000).filter(|_| Chance::percent(25).roll(&mut p)).count();
        assert!((9_500..10_500).contains(&hits), "{hits}");
    }

    #[test]
    fn display() {
        assert_eq!(Chance::per_mille(3).to_string(), "3/1000");
    }
}
