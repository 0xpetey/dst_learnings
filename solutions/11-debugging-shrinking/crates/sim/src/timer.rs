//! A timer that follows a state machine's deadline.
//!
//! The KV client does not own a timer. It reports the time it next needs
//! waking through `Client::next_deadline()`, and that value changes whenever
//! a request starts, retries or completes. The simulator's job is to keep one
//! scheduled event in step with that value. [`Timer::reconcile`] does exactly
//! that and nothing more: if the wanted deadline is the armed deadline, do
//! nothing; otherwise cancel the old event and schedule the new one.
//!
//! The same helper serves any component with a "next deadline" in later
//! modules: a node's heartbeat, an election timeout, a lease.

use crate::scheduler::{EventId, Scheduler};
use kv_core::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timer {
    armed: Option<(Instant, EventId)>,
}

impl Timer {
    pub const fn new() -> Timer {
        Timer { armed: None }
    }

    /// The deadline currently scheduled, if any.
    pub fn deadline(&self) -> Option<Instant> {
        self.armed.map(|(at, _)| at)
    }

    /// Bring the scheduled event in line with `want`. `make_event` is only
    /// called when a new event is needed.
    pub fn reconcile<E>(
        &mut self,
        scheduler: &mut Scheduler<E>,
        want: Option<Instant>,
        make_event: impl FnOnce() -> E,
    ) {
        if self.deadline() == want {
            return;
        }
        if let Some((_, id)) = self.armed.take() {
            scheduler.cancel(id);
        }
        if let Some(at) = want {
            let id = scheduler.schedule_at(at, make_event());
            self.armed = Some((at, id));
        }
    }

    /// Record that the event `id` fired. Returns true if it was the armed
    /// event. False means a stale event reached the handler, which cannot
    /// happen when every change goes through `reconcile`, so callers assert.
    pub fn fired(&mut self, id: EventId) -> bool {
        match self.armed {
            Some((_, armed)) if armed == id => {
                self.armed = None;
                true
            }
            _ => false,
        }
    }
}
