//! Environments for the KV store.
//!
//! * [`fake`]: everything in memory, time is a counter you advance by hand,
//!   storage completes when you say so. Used by tests now and by the
//!   simulator from Module 4 on.
//! * [`real`]: TCP, files with `fsync`, and the system clock. Used by the
//!   `kv` binary.
//!
//! Both drive the *same* `kv_core::Node` through the same `Driver`. There is
//! no `#[cfg(test)]` in the core and no mock framework: the fake is just
//! another implementation of the environment traits.

pub mod fake;
pub mod real;
