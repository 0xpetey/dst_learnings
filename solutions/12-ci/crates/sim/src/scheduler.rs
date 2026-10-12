//! The discrete-event scheduler.
//!
//! ```text
//!      schedule_at(t, e)                pop()
//!   ─────────────────────►  ┌──────────────────┐  ──────────────────►
//!                           │  (t, seq) → e    │   now := t
//!                           │  (t, seq) → e    │   return e
//!                           │  ...   min-heap  │
//!                           └──────────────────┘
//! ```
//!
//! Two rules make the queue deterministic, and both are easy to get wrong:
//!
//! 1. The heap key is `(at, seq)` and nothing else. `seq` is a counter that
//!    increments on every `schedule`. Two events at the same virtual time
//!    pop in the order they were scheduled. If the event payload took part
//!    in the ordering, a change to a `Debug` string or a field order would
//!    silently change the schedule. If nothing broke the tie, `BinaryHeap`
//!    would pick, and its choice is unspecified.
//! 2. Time only moves forwards, and only to the timestamp of the event being
//!    popped. You cannot schedule an event in the past. That is an assertion,
//!    not a clamp, because an event scheduled in the past is always a bug in
//!    the caller.
//!
//! Cancellation is lazy. The scheduler keeps the set of live ids beside the
//! heap. [`Scheduler::cancel`] removes an id from the set, and
//! [`Scheduler::pop`] discards heap entries whose id is no longer live as it
//! meets them. A `BinaryHeap` cannot remove from the middle, and this way it
//! never has to.

use crate::clock::Clock;
use alloc::collections::{BTreeSet, BinaryHeap};
use core::cmp::{Ordering, Reverse};
use kv_core::{Duration, Instant};

/// Identifies one scheduled event, for cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(u64);

/// A heap entry. `Ord` looks at `at` then `seq`, never at the event.
struct Slot<E> {
    at: Instant,
    seq: u64,
    event: E,
}

impl<E> PartialEq for Slot<E> {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.seq) == (other.at, other.seq)
    }
}
impl<E> Eq for Slot<E> {}
impl<E> PartialOrd for Slot<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<E> Ord for Slot<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.at, self.seq).cmp(&(other.at, other.seq))
    }
}

/// An event that the scheduler just popped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fired<E> {
    pub at: Instant,
    pub id: EventId,
    pub event: E,
}

/// Counters, for tests and for the end-of-run summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchedulerStats {
    pub scheduled: u64,
    pub fired: u64,
    pub cancelled: u64,
}

pub struct Scheduler<E> {
    clock: Clock,
    heap: BinaryHeap<Reverse<Slot<E>>>,
    /// Sequence numbers of events that are scheduled and not cancelled.
    live: BTreeSet<u64>,
    next_seq: u64,
    stats: SchedulerStats,
}

impl<E> Default for Scheduler<E> {
    fn default() -> Self {
        Scheduler::new()
    }
}

impl<E> Scheduler<E> {
    /// An empty scheduler with the clock at `Instant::ZERO`.
    pub fn new() -> Scheduler<E> {
        Scheduler::starting_at(Instant::ZERO)
    }

    pub fn starting_at(start: Instant) -> Scheduler<E> {
        Scheduler {
            clock: Clock::new(start),
            heap: BinaryHeap::new(),
            live: BTreeSet::new(),
            next_seq: 0,
            stats: SchedulerStats::default(),
        }
    }

    /// A handle that reads this scheduler's clock. Hand one to every
    /// component that needs the time.
    pub fn clock(&self) -> Clock {
        self.clock.clone()
    }

    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    pub fn stats(&self) -> SchedulerStats {
        self.stats
    }

    /// Live events: scheduled, not fired, not cancelled.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// Schedule `event` for virtual time `at`. Panics if `at` is in the past.
    /// `at == now` is allowed and means "after everything already scheduled
    /// for now".
    pub fn schedule_at(&mut self, at: Instant, event: E) -> EventId {
        assert!(
            at >= self.now(),
            "event scheduled in the past: at {at}, now {}",
            self.now()
        );
        let seq = self.next_seq;
        self.next_seq += 1;
        self.stats.scheduled += 1;
        self.heap.push(Reverse(Slot { at, seq, event }));
        self.live.insert(seq);
        EventId(seq)
    }

    /// Schedule `event` for `now + delay`.
    pub fn schedule_after(&mut self, delay: Duration, event: E) -> EventId {
        self.schedule_at(self.now() + delay, event)
    }

    /// Cancel a pending event. Returns false if it already fired or was
    /// already cancelled.
    pub fn cancel(&mut self, id: EventId) -> bool {
        let was_live = self.live.remove(&id.0);
        if was_live {
            self.stats.cancelled += 1;
        }
        was_live
    }

    /// Virtual time of the next live event, without popping it.
    pub fn peek(&mut self) -> Option<Instant> {
        self.reap();
        self.heap.peek().map(|Reverse(s)| s.at)
    }

    /// Pop the earliest event and move the clock to its time.
    pub fn pop(&mut self) -> Option<Fired<E>> {
        self.reap();
        let Reverse(slot) = self.heap.pop()?;
        self.live.remove(&slot.seq);
        self.clock.set(slot.at);
        self.stats.fired += 1;
        Some(Fired {
            at: slot.at,
            id: EventId(slot.seq),
            event: slot.event,
        })
    }

    /// Pop the earliest event if it is due at or before `limit`.
    pub fn pop_until(&mut self, limit: Instant) -> Option<Fired<E>> {
        match self.peek() {
            Some(at) if at <= limit => self.pop(),
            _ => None,
        }
    }

    /// Move the clock to `to` when nothing is due before it. Use this at the
    /// end of a bounded run so that `now()` reports the bound and not the
    /// last event. Panics if an event is due before `to`: skipping events is
    /// never what you want.
    pub fn advance_to(&mut self, to: Instant) {
        if let Some(at) = self.peek() {
            assert!(at >= to, "advance_to({to}) would skip an event due at {at}");
        }
        if to > self.now() {
            self.clock.set(to);
        }
    }

    /// Drop cancelled events from the top of the heap.
    fn reap(&mut self) {
        while let Some(Reverse(top)) = self.heap.peek() {
            if self.live.contains(&top.seq) {
                break;
            }
            self.heap.pop();
        }
    }
}
