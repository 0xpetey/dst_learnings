//! Planted bugs, for proving that the checker can find them.
//!
//! A checker that has never failed has never been tested. Each [`Bug`]
//! is a plausible mistake in the node, written as a branch that is taken
//! only when the bug is planted. The acceptance tests plant each one and
//! require the checker to catch it on some seed. That is the whole point
//! of the module: an oracle you have not seen fail is a hope, not a check.
//!
//! The mechanism is the one Module 7 built for fault points. Without the
//! `bugs` cargo feature, [`Bugs`] is zero sized and [`Bugs::has`] is a
//! constant `false`, so the production binary carries no bug at all. With
//! the feature, nothing is planted unless the simulator asks.

use core::fmt;

/// A mistake the node can be made to have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bug {
    /// A write to a key that already has a write in flight is logged and
    /// acknowledged but never applied, as if the one in flight covered
    /// it. The map ends up one write behind the log on that key. It takes
    /// two clients to reach, because one client has one request in flight.
    LostUpdate,
    /// Recovery records the session of the last record in the log but
    /// does not apply its entry. The node answers from a state one write
    /// behind what it acknowledged before the crash.
    StaleReadAfterRestart,
    /// A retry of a request that already finished is executed again
    /// instead of answered from the session. A deleted key is deleted
    /// twice; the second answer says it was never there.
    DuplicateApplyOnRetry,
}

impl Bug {
    pub const ALL: [Bug; 3] = [
        Bug::LostUpdate,
        Bug::StaleReadAfterRestart,
        Bug::DuplicateApplyOnRetry,
    ];
    pub const COUNT: usize = Bug::ALL.len();

    pub const fn name(self) -> &'static str {
        match self {
            Bug::LostUpdate => "lost_update",
            Bug::StaleReadAfterRestart => "stale_read_after_restart",
            Bug::DuplicateApplyOnRetry => "duplicate_apply_on_retry",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    pub fn from_name(name: &str) -> Option<Bug> {
        Bug::ALL.into_iter().find(|b| b.name() == name)
    }
}

impl fmt::Display for Bug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(feature = "bugs")]
mod enabled {
    use super::Bug;
    use alloc::vec::Vec;

    /// The bugs planted in one node.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Bugs {
        planted: [bool; Bug::COUNT],
    }

    impl Bugs {
        /// No bug planted. The production configuration.
        pub const fn none() -> Bugs {
            Bugs {
                planted: [false; Bug::COUNT],
            }
        }

        pub fn plant(&mut self, bug: Bug) {
            self.planted[bug.index()] = true;
        }

        /// Is `bug` planted? The branches in the node ask this.
        #[must_use]
        pub const fn has(&self, bug: Bug) -> bool {
            self.planted[bug.index()]
        }

        pub fn planted(&self) -> Vec<Bug> {
            Bug::ALL.into_iter().filter(|b| self.has(*b)).collect()
        }
    }
}

#[cfg(not(feature = "bugs"))]
mod disabled {
    use super::Bug;
    use alloc::vec::Vec;

    /// Zero sized. [`Bugs::has`] is a constant `false` that the compiler
    /// removes along with the branch.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Bugs;

    impl Bugs {
        pub const fn none() -> Bugs {
            Bugs
        }

        #[inline(always)]
        #[must_use]
        pub const fn has(&self, _bug: Bug) -> bool {
            false
        }

        pub fn planted(&self) -> Vec<Bug> {
            Vec::new()
        }
    }
}

#[cfg(not(feature = "bugs"))]
pub use disabled::Bugs;
#[cfg(feature = "bugs")]
pub use enabled::Bugs;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn names_are_unique_and_match_display() {
        for (i, a) in Bug::ALL.iter().enumerate() {
            assert_eq!(a.index(), i);
            assert_eq!(a.to_string(), a.name());
            assert_eq!(Bug::from_name(a.name()), Some(*a));
            for b in &Bug::ALL[i + 1..] {
                assert_ne!(a.name(), b.name());
            }
        }
        assert_eq!(Bug::from_name("nope"), None);
    }

    #[test]
    fn none_has_nothing() {
        let b = Bugs::none();
        assert!(Bug::ALL.iter().all(|bug| !b.has(*bug)));
        assert!(b.planted().is_empty());
    }
}
