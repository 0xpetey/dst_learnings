//! Determinism primitives for the DST course. Module 1 fills this in.
//!
//! Three things live here, and every later module builds on them:
//!
//! * [`Prng`]: a seedable pseudo-random number generator with a *pinned*
//!   algorithm. The seed is the single root of all randomness in a
//!   simulation: `seed -> Prng -> every decision`.
//! * [`Trace`]: an append-only log of events with a running hash. Two runs
//!   are "the same" exactly when their trace hashes are equal.
//! * [`harness`]: the determinism check. Run a program twice under one seed,
//!   compare the traces, and report the first divergence.
//!
//! The signatures below are the contract the tests in `tests/` and every
//! later module rely on. Replace each `todo!()` and keep the signatures.

pub mod harness;
pub mod prng;
pub mod trace;

pub use harness::{check_determinism, run_twice, Divergence};
pub use prng::Prng;
pub use trace::Trace;
