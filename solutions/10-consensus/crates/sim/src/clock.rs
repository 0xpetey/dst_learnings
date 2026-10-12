//! The virtual clock, and each host's imperfect view of it.
//!
//! The scheduler owns time. Everything else that needs to read the time
//! holds a [`Clock`], a cheap handle onto the scheduler's current instant.
//! A `Clock` can only be read. Only [`Scheduler::pop`] and
//! [`Scheduler::advance_to`] move it, and only forwards.
//!
//! A real machine does not read the scheduler's clock. It reads its own
//! oscillator, which runs a little fast or slow, started from a different
//! zero, and now and then leaps forward when a hypervisor pauses the guest.
//! [`HostClock`] is that view:
//!
//! ```text
//!   local(t) = offset + t * (1_000_000 + drift_ppm) / 1_000_000
//!
//!   true  ──────────────────────────────────►
//!   n0    ──────────────────────────────────►   drift_ppm = 0
//!   n1    ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─►   drift_ppm > 0 (fast)
//!   n2    ────────────╱──────────────────────►  jump: offset grows
//! ```
//!
//! Only the process on the host reads its `HostClock`. The disk, the
//! network and the scheduler keep true time, because the hardware does not
//! care what the kernel thinks the time is. The world converts a host's
//! local deadline back to true time with [`HostClock::to_true`] when it
//! arms a timer for it.
//!
//! The clock is monotonic. Drift and jumps only move it forwards, and
//! there is no backward step, because [`Instant`] models a monotonic clock
//! and a backward step on one is a hardware bug the core may assert
//! against. TigerBeetle's simulated time keeps its monotonic clock ideal
//! and skews only the wall clock. This model skews the monotonic clock
//! too, because the heartbeat's timeouts are measured on it, and two
//! monotonic clocks on two machines do drift apart.
//!
//! The handle is an `Rc<Cell<_>>`, which is single-threaded by construction.
//! That is a feature: a clock that two threads could read is a clock that
//! two threads could race on.
//!
//! [`Scheduler::pop`]: crate::Scheduler::pop
//! [`Scheduler::advance_to`]: crate::Scheduler::advance_to

use alloc::rc::Rc;
use core::cell::Cell;
use core::fmt;
use determinism::Prng;
use kv_core::env::Time;
use kv_core::{Duration, Instant};

/// A read-only handle onto the simulator's clock.
#[derive(Clone, Debug)]
pub struct Clock {
    now: Rc<Cell<Instant>>,
}

impl Clock {
    pub(crate) fn new(start: Instant) -> Clock {
        Clock {
            now: Rc::new(Cell::new(start)),
        }
    }

    pub fn now(&self) -> Instant {
        self.now.get()
    }

    /// Only the scheduler moves the clock, and only forwards.
    pub(crate) fn set(&self, to: Instant) {
        debug_assert!(to >= self.now.get(), "virtual time moved backwards");
        self.now.set(to);
    }
}

impl Time for Clock {
    fn now(&self) -> Instant {
        Clock::now(self)
    }
}

/// Largest drift the model accepts, in parts per million: 50%. Real
/// oscillators are within 500 ppm. The simulator exaggerates on purpose,
/// and the lesson says how far is useful.
pub const DRIFT_PPM_MAX: i64 = 500_000;

/// How one host's clock differs from true time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skew {
    /// Local time at true time zero, in nanoseconds, plus every jump
    /// since. Signed, because a rate change that keeps the clock
    /// continuous can need a negative one. The clock itself never reads
    /// below zero.
    pub offset_nanos: i64,
    /// Rate error in parts per million. Positive runs fast.
    pub drift_ppm: i64,
}

impl Skew {
    pub const IDEAL: Skew = Skew {
        offset_nanos: 0,
        drift_ppm: 0,
    };

    /// A clock that starts `offset` ahead and runs true.
    pub fn ahead(offset: Duration) -> Skew {
        Skew {
            offset_nanos: i64::try_from(offset.as_nanos()).expect("offset fits i64"),
            drift_ppm: 0,
        }
    }

    pub fn validate(&self) {
        assert!(
            self.drift_ppm.abs() <= DRIFT_PPM_MAX,
            "drift of {} ppm is outside the model",
            self.drift_ppm
        );
    }
}

impl fmt::Display for Skew {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.offset_nanos < 0 { "-" } else { "+" };
        let offset = Duration::from_nanos(self.offset_nanos.unsigned_abs());
        write!(f, "offset={sign}{offset:?} drift={}ppm", self.drift_ppm)
    }
}

/// The clock fault model for a world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockConfig {
    /// Each host starts with an offset uniform in `0..=offset_max`.
    pub offset_max: Duration,
    /// Each host drifts by a rate uniform in `-drift_ppm_max..=drift_ppm_max`.
    pub drift_ppm_max: i64,
    /// Jumps: after a gap uniform in `0..=jump_gap_max`, some host's clock
    /// leaps forward by `0..=jump_max`. `jump_max` of zero means no jumps.
    pub jump_gap_max: Duration,
    pub jump_max: Duration,
}

impl ClockConfig {
    /// Every host reads true time.
    pub const IDEAL: ClockConfig = ClockConfig {
        offset_max: Duration::ZERO,
        drift_ppm_max: 0,
        jump_gap_max: Duration::ZERO,
        jump_max: Duration::ZERO,
    };

    /// A configuration drawn from the seed, in a fixed order.
    pub fn random(prng: &mut Prng) -> ClockConfig {
        let offset_max = Duration::from_millis(prng.below(2000));
        let drift_ppm_max = prng.below(100_001) as i64;
        let jumps = prng.chance(1, 2);
        let (jump_gap_max, jump_max) = if jumps {
            (
                Duration::from_millis(prng.range(100, 5000)),
                Duration::from_millis(prng.range(1, 1000)),
            )
        } else {
            (Duration::ZERO, Duration::ZERO)
        };
        ClockConfig {
            offset_max,
            drift_ppm_max,
            jump_gap_max,
            jump_max,
        }
    }

    pub fn has_jumps(&self) -> bool {
        !self.jump_max.is_zero()
    }

    pub fn validate(&self) {
        assert!(
            (0..=DRIFT_PPM_MAX).contains(&self.drift_ppm_max),
            "drift_ppm_max {} is outside the model",
            self.drift_ppm_max
        );
    }

    /// Draw one host's skew.
    pub fn draw_skew(&self, prng: &mut Prng) -> Skew {
        let offset_nanos = if self.offset_max.is_zero() {
            0
        } else {
            let max = u64::try_from(self.offset_max.as_nanos()).expect("offset fits u64");
            i64::try_from(prng.below(max + 1)).expect("offset fits i64")
        };
        let drift_ppm = if self.drift_ppm_max == 0 {
            0
        } else {
            let span = 2 * self.drift_ppm_max as u64 + 1;
            prng.below(span) as i64 - self.drift_ppm_max
        };
        Skew {
            offset_nanos,
            drift_ppm,
        }
    }
}

impl fmt::Display for ClockConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "offset<={:?} drift<={}ppm",
            self.offset_max, self.drift_ppm_max
        )?;
        if self.has_jumps() {
            write!(f, " jumps every<={:?} by<={:?}", self.jump_gap_max, self.jump_max)
        } else {
            write!(f, " no jumps")
        }
    }
}

/// One host's clock: the true clock seen through a [`Skew`]. Clones share
/// the skew, so the world and the host's process read the same clock.
#[derive(Clone, Debug)]
pub struct HostClock {
    base: Clock,
    skew: Rc<Cell<Skew>>,
}

const MILLION: i128 = 1_000_000;

impl HostClock {
    pub fn new(base: Clock, skew: Skew) -> HostClock {
        skew.validate();
        HostClock {
            base,
            skew: Rc::new(Cell::new(skew)),
        }
    }

    /// A host whose clock is the true clock.
    pub fn ideal(base: Clock) -> HostClock {
        HostClock::new(base, Skew::IDEAL)
    }

    pub fn skew(&self) -> Skew {
        self.skew.get()
    }

    /// What this host's clock reads now.
    pub fn now(&self) -> Instant {
        self.local(self.base.now())
    }

    /// What this host's clock reads at true time `t`. Never below zero.
    pub fn local(&self, t: Instant) -> Instant {
        let skew = self.skew.get();
        let scaled = i128::from(t.nanos()) * (MILLION + i128::from(skew.drift_ppm)) / MILLION;
        let local = (i128::from(skew.offset_nanos) + scaled).max(0);
        Instant::from_nanos(u64::try_from(local).expect("local clock overflowed"))
    }

    /// The earliest true time at which this clock reads `local` or later.
    /// A timer armed for a local deadline fires here.
    pub fn to_true(&self, local: Instant) -> Instant {
        let skew = self.skew.get();
        let past_offset = i128::from(local.nanos()) - i128::from(skew.offset_nanos);
        if past_offset <= 0 {
            return Instant::ZERO;
        }
        let rate = MILLION + i128::from(skew.drift_ppm);
        // ceil(past_offset * MILLION / rate)
        let mut t = (past_offset * MILLION + rate - 1) / rate;
        let mut at = Instant::from_nanos(u64::try_from(t).expect("true time fits u64"));
        // Integer division is exact enough that these loops never run more
        // than once. They are here so that the contract is checked by
        // construction, not by arithmetic that a reader has to trust.
        while self.local(at) < local {
            t += 1;
            at = Instant::from_nanos(t as u64);
        }
        while t > 0 && self.local(Instant::from_nanos(t as u64 - 1)) >= local {
            t -= 1;
            at = Instant::from_nanos(t as u64);
        }
        at
    }

    /// The clock leaps forward by `by`. A hypervisor paused the guest, or
    /// the kernel caught up with a missed interrupt.
    pub fn jump(&self, by: Duration) {
        let mut skew = self.skew.get();
        skew.offset_nanos += i64::try_from(by.as_nanos()).expect("jump fits i64");
        self.skew.set(skew);
    }

    /// Change the rate without moving the clock: the reading at this
    /// instant is the same before and after. Only the future runs at the
    /// new rate.
    pub fn set_drift_ppm(&self, drift_ppm: i64) {
        let before = self.now();
        let t = self.base.now();
        let mut skew = self.skew.get();
        skew.drift_ppm = drift_ppm;
        skew.validate();
        let scaled = i128::from(t.nanos()) * (MILLION + i128::from(drift_ppm)) / MILLION;
        skew.offset_nanos = i64::try_from(i128::from(before.nanos()) - scaled).expect("offset fits i64");
        self.skew.set(skew);
        debug_assert_eq!(self.now(), before);
    }
}

impl Time for HostClock {
    fn now(&self) -> Instant {
        HostClock::now(self)
    }
}
