//! The simulator.
//!
//! Module 4 added the heart of it: a virtual clock and a discrete-event
//! scheduler. Module 5 adds the network model in [`net`]. Everything that happens in a simulated run is an *event* with
//! a virtual timestamp. The scheduler keeps the events in a priority queue,
//! pops the earliest, jumps the clock to its timestamp, and hands it to
//! whoever owns the world. Nothing ever sleeps, so a simulated year of idle
//! time costs one heap pop.
//!
//! Later modules add a disk model and fault injection on
//! top. They are all the same shape: something schedules an event, and
//! something handles it when the clock reaches it.
//!
//! The crate is `no_std` for the same reason `kv-core` is: a simulator that
//! reads the real clock, or iterates a `HashMap`, is not a simulator. The
//! only `std` in the system lives in `kv-env`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![deny(
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::ptr_as_ptr,
    clippy::fn_to_numeric_cast_any
)]

extern crate alloc;

pub mod clock;
pub mod net;
pub mod scheduler;
pub mod timer;

pub use clock::Clock;
pub use kv_core::{Duration, Instant};
pub use net::{Chance, Delivery, Dropped, MessageId, Net, NetConfig, NetStats, Shape};
pub use scheduler::{EventId, Fired, Scheduler, SchedulerStats};
pub use timer::Timer;
