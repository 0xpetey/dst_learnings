//! BUGGIFY: fault points inside the code under test.
//!
//! The network and disk models inject faults from outside. They cannot
//! reach a decision the node makes on its own: whether to send a reply,
//! whether a buffer has room, whether this tick sends a beat. FoundationDB's
//! answer is `BUGGIFY`, a macro at the point of decision that is false in
//! production and sometimes true under simulation. This module is the
//! Rust version.
//!
//! ```text
//!   Buggify::new(sites, fire, prng)            buggify!(self.buggify, Site::DropReply)
//!   ┌──────────────────────────────┐              │
//!   │ enabled: [bool; sites]       │ ◄────────────┘ enabled[site] && fire.roll(prng)
//!   │ fire:    Chance              │
//!   │ prng:    Prng                │ ──► true: the code takes the unusual path
//!   │ fired:   [u64; sites]        │ ──► false: the code does what it always does
//!   └──────────────────────────────┘
//! ```
//!
//! Two probabilities, as in FoundationDB. A *site* is enabled or disabled
//! for a whole run; the simulator draws the set once from the seed. An
//! enabled site then *fires* with a fixed probability each time the code
//! reaches it. A disabled site never fires, so a run with a different
//! enabled set exercises a different mix of unusual paths. That is swarm
//! testing applied to the code's own decisions, and the lesson says why
//! it beats enabling everything everywhere.
//!
//! Every site has a name in [`Site`], so the trace can say which one fired
//! and a test can enable exactly one. `grep -rn 'buggify!' src` lists every
//! fault point in the crate. That is the whole discovery mechanism, and it
//! is the same one FoundationDB has.
//!
//! Without the `buggify` cargo feature, [`Buggify`] is a zero-sized type
//! and [`Buggify::fire`] is a constant `false`. The compiler removes the
//! branch, so the production binary carries no dice and no site table.
//! With the feature, the production server still constructs
//! [`Buggify::off`], under which no site is enabled. Two layers, both
//! deliberate: a fault point that could fire in production is a bug.

use core::fmt;

/// A fault point. One variant per `buggify!` in the crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Site {
    /// The node pretends its inbound buffer was full and drops a request
    /// unread. The client retries.
    DropRequest,
    /// The node pretends the outbound buffer was full and sends no reply.
    /// The client retries, and the session answers the retry.
    DropReply,
    /// The node sends a reply twice. The client must ignore the second.
    DuplicateReply,
    /// The heartbeat misses one round, as a process does when it stalls
    /// for a moment. Peers must tolerate a late beat.
    SkipBeat,
}

impl Site {
    pub const ALL: [Site; 4] = [
        Site::DropRequest,
        Site::DropReply,
        Site::DuplicateReply,
        Site::SkipBeat,
    ];
    pub const COUNT: usize = Site::ALL.len();

    pub const fn name(self) -> &'static str {
        match self {
            Site::DropRequest => "drop_request",
            Site::DropReply => "drop_reply",
            Site::DuplicateReply => "duplicate_reply",
            Site::SkipBeat => "skip_beat",
        }
    }

    /// Position in `Site::ALL`, for the tables in [`Buggify`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// `buggify!(b, site)`: true when `site` is enabled in `b` and the dice say
/// so. Expands to a method call, so that it reads as a marker and `grep`
/// finds every fault point. Without the `buggify` feature the method is a
/// constant `false` and the branch disappears.
///
/// Why not a `#[cfg]` inside the macro? A `cfg` in a `macro_rules!` body is
/// evaluated in the crate that *expands* the macro, against that crate's
/// features. The gate belongs on the method, in this crate.
#[macro_export]
macro_rules! buggify {
    ($b:expr, $site:expr) => {
        $crate::buggify::Buggify::fire(&mut $b, $site)
    };
}

#[cfg(feature = "buggify")]
mod enabled {
    use super::Site;
    use alloc::vec::Vec;
    use determinism::{Chance, Prng};

    /// The fault-point state of one state machine: which sites are
    /// enabled, how often they fire, the dice, and the counts.
    #[derive(Clone, Debug)]
    pub struct Buggify {
        enabled: [bool; Site::COUNT],
        fire: Chance,
        /// `None` under [`Buggify::off`], which has nothing to roll.
        prng: Option<Prng>,
        fired: [u64; Site::COUNT],
        /// Sites that fired since the last [`Buggify::take_log`].
        log: Vec<Site>,
    }

    impl Buggify {
        /// No site enabled. The production configuration.
        pub const fn off() -> Buggify {
            Buggify {
                enabled: [false; Site::COUNT],
                fire: Chance::NEVER,
                prng: None,
                fired: [0; Site::COUNT],
                log: Vec::new(),
            }
        }

        /// Enable `sites`, each firing with probability `fire`.
        pub fn new(sites: &[Site], fire: Chance, prng: Prng) -> Buggify {
            let mut b = Buggify::off();
            for &site in sites {
                b.enabled[site.index()] = true;
            }
            b.fire = fire;
            b.prng = Some(prng);
            b
        }

        /// Draw the enabled set for a run: each site with probability
        /// `enable`, in `Site::ALL` order.
        pub fn draw_sites(enable: Chance, prng: &mut Prng) -> Vec<Site> {
            Site::ALL.iter().copied().filter(|_| enable.roll(prng)).collect()
        }

        pub fn is_enabled(&self, site: Site) -> bool {
            self.enabled[site.index()]
        }

        pub fn enabled_sites(&self) -> Vec<Site> {
            Site::ALL
                .iter()
                .copied()
                .filter(|s| self.is_enabled(*s))
                .collect()
        }

        pub fn fire_chance(&self) -> Chance {
            self.fire
        }

        /// Change the firing probability. Tests use it to fire once and
        /// then stop.
        pub fn set_fire(&mut self, fire: Chance) {
            self.fire = fire;
        }

        /// Disable every site. The quiet period of a run.
        pub fn disable_all(&mut self) {
            self.enabled = [false; Site::COUNT];
        }

        /// The decision. Spends randomness only on an enabled site.
        #[must_use]
        pub fn fire(&mut self, site: Site) -> bool {
            if !self.enabled[site.index()] {
                return false;
            }
            let Some(prng) = self.prng.as_mut() else {
                return false;
            };
            if !self.fire.roll(prng) {
                return false;
            }
            self.fired[site.index()] += 1;
            self.log.push(site);
            true
        }

        /// How often `site` fired.
        pub fn fired(&self, site: Site) -> u64 {
            self.fired[site.index()]
        }

        pub fn total_fired(&self) -> u64 {
            self.fired.iter().sum()
        }

        /// The sites that fired since the last call, in order. The
        /// simulator drains this into the trace.
        pub fn take_log(&mut self) -> Vec<Site> {
            core::mem::take(&mut self.log)
        }
    }
}

#[cfg(not(feature = "buggify"))]
mod disabled {
    use super::Site;
    use alloc::vec::Vec;

    /// Zero sized. Every query says "nothing", and [`Buggify::fire`] is a
    /// constant `false` that the compiler removes along with the branch.
    #[derive(Clone, Copy, Debug)]
    pub struct Buggify;

    impl Buggify {
        pub const fn off() -> Buggify {
            Buggify
        }

        #[inline(always)]
        #[must_use]
        pub const fn fire(&mut self, _site: Site) -> bool {
            false
        }

        pub const fn fired(&self, _site: Site) -> u64 {
            0
        }

        pub const fn total_fired(&self) -> u64 {
            0
        }

        pub fn take_log(&mut self) -> Vec<Site> {
            Vec::new()
        }
    }
}

#[cfg(not(feature = "buggify"))]
pub use disabled::Buggify;
#[cfg(feature = "buggify")]
pub use enabled::Buggify;

impl Default for Buggify {
    fn default() -> Buggify {
        Buggify::off()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn names_are_unique_and_match_display() {
        for (i, a) in Site::ALL.iter().enumerate() {
            assert_eq!(a.index(), i);
            assert_eq!(a.to_string(), a.name());
            for b in &Site::ALL[i + 1..] {
                assert_ne!(a.name(), b.name());
            }
        }
    }

    #[test]
    fn off_never_fires() {
        let mut b = Buggify::off();
        for _ in 0..100 {
            for site in Site::ALL {
                assert!(!buggify!(b, site));
            }
        }
        assert_eq!(b.total_fired(), 0);
        assert!(b.take_log().is_empty());
    }
}
