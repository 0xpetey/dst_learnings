//! Time as the core sees it.
//!
//! `std::time::Instant` cannot be used here even though we are allowed `std`
//! in Module 2: there is no way to *construct* one except by asking the OS
//! for "now", so a fake clock could never mint an `Instant` for the year
//! 2031. The core needs its own notion of a point in time that is just a
//! number. The environment decides what that number means.

/// Re-exported: `Duration` is a plain value type and lives in `core`.
pub use core::time::Duration;

/// A point in time, in nanoseconds since an environment-defined epoch.
///
/// Comparison and subtraction are meaningful; the absolute value is not.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant {
    nanos: u64,
}

impl Instant {
    /// The environment's epoch.
    pub const ZERO: Instant = Instant { nanos: 0 };

    pub const fn from_nanos(nanos: u64) -> Instant {
        Instant { nanos }
    }

    pub const fn nanos(self) -> u64 {
        self.nanos
    }

    /// `self + d`, or `None` on overflow. Overflow is an error, never a wrap:
    /// a simulated clock that wraps would silently reorder every timer.
    pub fn checked_add(self, d: Duration) -> Option<Instant> {
        let d = u64::try_from(d.as_nanos()).ok()?;
        self.nanos.checked_add(d).map(Instant::from_nanos)
    }

    /// Time elapsed from `earlier` to `self`, or zero if `earlier` is later.
    pub fn saturating_duration_since(self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.nanos.saturating_sub(earlier.nanos))
    }
}

impl core::ops::Add<Duration> for Instant {
    type Output = Instant;
    /// Panics on overflow, in every profile.
    fn add(self, d: Duration) -> Instant {
        self.checked_add(d).expect("Instant + Duration overflowed")
    }
}

impl core::fmt::Display for Instant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "t={}ns", self.nanos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let t = Instant::from_nanos(10);
        assert_eq!(t + Duration::from_nanos(5), Instant::from_nanos(15));
        assert_eq!(
            (t + Duration::from_nanos(5)).saturating_duration_since(t),
            Duration::from_nanos(5)
        );
        assert_eq!(
            t.saturating_duration_since(t + Duration::from_secs(1)),
            Duration::ZERO
        );
        assert_eq!(
            Instant::from_nanos(u64::MAX).checked_add(Duration::from_nanos(1)),
            None
        );
    }

    #[test]
    #[should_panic(expected = "overflowed")]
    fn add_overflow_panics() {
        let _ = Instant::from_nanos(u64::MAX) + Duration::from_nanos(1);
    }
}
