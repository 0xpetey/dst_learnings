//! The oracle problem, and three answers to it.
//!
//! The simulator of Modules 4 to 7 can make the store suffer. It cannot
//! say whether an answer the store gave was *wrong*. That is the oracle
//! problem, and this crate holds the oracles:
//!
//! ```text
//!   workload ──► clients ──► the store ──► replies
//!       │                                    │
//!       └──────► history (call, return) ◄────┘
//!                     │
//!        ┌────────────┼────────────────┐
//!   sequential?    concurrent?     the node's own state
//!   check_sequential  linearize::check   kv::replay vs Node::entries
//!   (the model)       (the model, every   (the log is the truth)
//!                      legal order)
//! ```
//!
//! * [`workload`] draws requests from the seed: an operation mix, a key
//!   space with a hot set, values that name the operation that wrote them,
//!   and think time between operations.
//! * [`history`] records what each client asked, when, and what it was
//!   told, when. A request the client gave up on stays open: its fate is
//!   unknown, and the checker has to allow both.
//! * [`model`] is the reference: a sequential specification that says what
//!   each operation returns. For a history with no overlap, comparing the
//!   model's answers with the store's is the whole check.
//! * [`linearize`] is the check for histories with overlap. An operation
//!   takes effect at some instant between its call and its return. The
//!   history is correct if there is some choice of those instants under
//!   which the model agrees with every reply. The search is Wing and
//!   Gong's, with Lowe's cache of configurations already seen.
//! * [`kv`] binds the generic parts to the KV store: the model, the
//!   partition by key that makes the search tractable, and a replay of
//!   the log for the white-box check.
//!
//! The crate is `no_std` like the simulator: an oracle that reads a clock
//! or iterates a `HashMap` has two opinions.

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

pub mod history;
pub mod kv;
pub mod linearize;
pub mod model;
pub mod workload;

pub use history::{History, Op};
pub use kv::KvModel;
pub use linearize::{Illegal, Stats, Verdict};
pub use model::{check_sequential, Model, SequentialError};
pub use workload::{Step, Workload, WorkloadConfig, WorkloadStats};
