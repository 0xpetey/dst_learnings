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
//! Module 2 lets this crate use `std`. Module 3 removes that permission.

pub mod client;
pub mod env;
pub mod log;
pub mod node;
pub mod protocol;
pub mod time;

pub use client::Client;
pub use node::Node;
pub use protocol::{ClientId, Request, RequestEnvelope, RequestId, Response, ResponseEnvelope};
pub use time::{Duration, Instant};
