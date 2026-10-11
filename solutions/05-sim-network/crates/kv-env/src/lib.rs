//! Environments for the KV store.
//!
//! * [`fake`]: everything in memory, on virtual time. A discrete-event
//!   scheduler from the `sim` crate owns the clock, and a network model from
//!   the same crate decides the fate of every message. Several hosts share
//!   one world. Used by tests, and the base of the simulator that later
//!   modules build.
//! * [`real`]: TCP, files with `fsync`, and the system clock. Used by the
//!   `kv` binary.
//!
//! Both drive the *same* `kv_core::Node` through the same `Driver`. There is
//! no `#[cfg(test)]` in the core and no mock framework: the fake is just
//! another implementation of the environment traits.

pub mod fake;
pub mod real;
