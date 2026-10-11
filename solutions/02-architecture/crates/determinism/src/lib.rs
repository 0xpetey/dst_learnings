//! Determinism primitives for the DST course.
//!
//! Three things live here, and every later module builds on them:
//!
//! * [`Prng`]: a seedable pseudo-random number generator with a *pinned*
//!   algorithm (xoshiro256++). The seed is the single root of all randomness
//!   in a simulation: `seed -> Prng -> every decision`.
//! * [`Trace`]: an append-only log of events with a running hash. Two runs
//!   are "the same" exactly when their trace hashes are equal.
//! * [`harness`]: the determinism check. Run a program twice under one seed,
//!   compare the traces, and report the first divergence.
//!
//! Nothing in this crate reads the clock, the environment, the filesystem or
//! the network, except the cross-process runner in [`harness`], which exists
//! precisely to spawn a program as a separate OS process.

pub mod harness;
pub mod prng;
pub mod trace;

pub use harness::{check_determinism, run_twice, Divergence};
pub use prng::Prng;
pub use trace::Trace;
