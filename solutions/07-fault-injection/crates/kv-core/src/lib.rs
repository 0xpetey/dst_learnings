//! The key-value store, as pure state machines.
//!
//! Nothing in this crate performs I/O, reads a clock, or spawns a thread.
//! The [`node::Node`] and [`client::Client`] consume *inputs* (a request
//! arrived, a write became durable, time passed) and produce *outputs*
//! (send this reply, append these bytes, read the log). Something else, an
//! *environment*, turns outputs into real or simulated effects and feeds the
//! resulting events back in. The traits in [`env`] describe that contract.
//!
//! This is the "sans-I/O" style. It is what makes the same code runnable
//! against a TCP socket and a disk in production and against an in-memory
//! fake under a simulator, with no `cfg` and no mocking framework.
//!
//! This crate is `no_std`. That is not about embedded targets: it is the
//! strongest statement Rust can make that `std::time`, `std::thread`,
//! `std::fs`, `std::net`, `std::env` and `HashMap`'s random hasher do not
//! exist here. Everything that could reach the OS is a compile error, not a
//! code-review item. `alloc` is allowed: `Vec`, `BTreeMap` and `String` are
//! deterministic.
//!
//! Module 7 added one thing that looks like I/O and is not: fault points.
//! `buggify!(self.buggify, Site::DropReply)` asks a seeded [`Buggify`]
//! whether to take an unusual path. In production nothing is enabled and,
//! without the `buggify` feature, the call is a constant `false`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
// Addresses differ per process under ASLR. `no_std` cannot stop a reference
// being cast to a pointer and then to an integer, so lint the first step.
#![deny(
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::ptr_as_ptr,
    clippy::fn_to_numeric_cast_any
)]

extern crate alloc;

pub mod buggify;
pub mod checksum;
pub mod client;
pub mod env;
pub mod heartbeat;
pub mod log;
pub mod node;
pub mod protocol;
pub mod time;

pub use buggify::{Buggify, Site};
pub use client::Client;
pub use heartbeat::{Heartbeat, NodeId};
pub use node::Node;
pub use protocol::{ClientId, Request, RequestEnvelope, RequestId, Response, ResponseEnvelope};
pub use time::{Duration, Instant};
