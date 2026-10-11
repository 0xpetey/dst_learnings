//! The virtual clock.
//!
//! The scheduler owns time. Everything else that needs to read the time
//! holds a [`Clock`], a cheap handle onto the scheduler's current instant.
//! A `Clock` can only be read. Only [`Scheduler::pop`] and
//! [`Scheduler::advance_to`] move it, and only forwards.
//!
//! The handle is an `Rc<Cell<_>>`, which is single-threaded by construction.
//! That is a feature: a clock that two threads could read is a clock that
//! two threads could race on.
//!
//! [`Scheduler::pop`]: crate::Scheduler::pop
//! [`Scheduler::advance_to`]: crate::Scheduler::advance_to

use alloc::rc::Rc;
use core::cell::Cell;
use kv_core::env::Time;
use kv_core::Instant;

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
