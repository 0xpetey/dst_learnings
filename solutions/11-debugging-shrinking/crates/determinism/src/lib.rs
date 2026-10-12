//! Determinism primitives for the DST course.
//!
//! Three things live here, and every later module builds on them:
//!
//! * [`Prng`]: a seedable pseudo-random number generator with a *pinned*
//!   algorithm (xoshiro256++). The seed is the single root of all randomness
//!   in a simulation: `seed -> Prng -> every decision`.
//! * [`Chance`]: a probability as a fraction, rolled against a `Prng`. Every
//!   fault in the simulator is one of these.
//! * [`Trace`]: an append-only log of events with a running hash. Two runs
//!   are "the same" exactly when their trace hashes are equal.
//! * [`harness`]: the determinism check. Run a program twice under one seed,
//!   compare the traces, and report the first divergence.
//!
//! Nothing in this crate reads the clock, the environment, the filesystem or
//! the network, except the cross-process runner in [`harness`], which exists
//! precisely to spawn a program as a separate OS process. That is why the
//! harness sits behind the `std` feature and the rest of the crate is
//! `no_std`: `Prng` and `Trace` can be used from the core crate, which has
//! no `std` at all.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod chance;
#[cfg(feature = "std")]
pub mod harness;
pub mod prng;
pub mod trace;

pub use chance::Chance;
#[cfg(feature = "std")]
pub use harness::{check_determinism, run_twice, Divergence};
pub use prng::Prng;
pub use trace::Trace;
