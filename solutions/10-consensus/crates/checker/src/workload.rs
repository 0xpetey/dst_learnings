//! The workload: what the clients ask for, drawn from the seed.
//!
//! A test that does one operation at a time, on a fresh key each time,
//! finds nothing. Bugs live where operations collide: two clients on one
//! key, a delete racing a put, a retry landing after the next request. So
//! the generator has a hot set of keys that most operations go to, an
//! operation mix with deletes in it, and clients that sometimes pause so
//! the number in flight varies.
//!
//! Values name the operation that wrote them, so the checker can tell
//! which write a read saw. A store that returned "the right value" could
//! have gotten it from the wrong write.
//!
//! The generator does not know about retries. The client does those, and
//! the history records one operation per request id however many times it
//! was sent.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use determinism::{Chance, Prng};
use kv_core::{ClientId, Duration, Request};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadConfig {
    /// Keys are `k0` to `k{keys - 1}`.
    pub keys: u32,
    /// The first `hot_keys` keys are the hot set.
    pub hot_keys: u32,
    /// Probability that an operation goes to the hot set.
    pub hot: Chance,
    /// Relative weights of the three operations.
    pub gets: u32,
    pub puts: u32,
    pub deletes: u32,
    /// Values are `1..=value_len_max` bytes.
    pub value_len_max: u32,
    /// Probability that a client thinks before its next operation.
    pub think: Chance,
    /// How long, at most.
    pub think_max: Duration,
}

impl Default for WorkloadConfig {
    fn default() -> WorkloadConfig {
        WorkloadConfig {
            keys: 16,
            hot_keys: 2,
            hot: Chance::percent(50),
            gets: 2,
            puts: 2,
            deletes: 1,
            value_len_max: 16,
            think: Chance::percent(25),
            think_max: Duration::from_millis(100),
        }
    }
}

impl WorkloadConfig {
    /// A workload drawn from the seed: a few keys or many, a hot set that
    /// takes most of the traffic or none, a read-heavy or write-heavy mix.
    pub fn random(prng: &mut Prng) -> WorkloadConfig {
        let keys = prng.range(1, 33) as u32;
        let hot_keys = prng.range(0, u64::from(keys) + 1).min(4) as u32;
        let hot = if hot_keys == 0 {
            Chance::NEVER
        } else {
            Chance::percent(prng.range(20, 91) as u32)
        };
        WorkloadConfig {
            keys,
            hot_keys,
            hot,
            gets: prng.range(1, 6) as u32,
            puts: prng.range(1, 6) as u32,
            deletes: prng.range(0, 4) as u32,
            value_len_max: prng.range(1, 65) as u32,
            think: Chance::percent(prng.range(0, 51) as u32),
            think_max: Duration::from_millis(prng.range(1, 301)),
        }
    }

    pub fn validate(&self) {
        assert!(self.keys >= 1, "a workload needs a key");
        assert!(self.hot_keys <= self.keys, "the hot set is a subset of the keys");
        assert!(self.hot_keys > 0 || self.hot.is_never(), "no hot keys to go to");
        assert!(
            self.gets + self.puts + self.deletes > 0,
            "a workload needs an operation"
        );
        assert!(self.value_len_max >= 1);
        assert!(self.think.is_never() || !self.think_max.is_zero());
    }

    pub fn key(index: u32) -> Vec<u8> {
        format!("k{index}").into_bytes()
    }
}

impl fmt::Display for WorkloadConfig {
    /// One line, for the trace.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "keys={} hot={}@{} mix=get:{} put:{} delete:{} value<={} think={}@<={:?}",
            self.keys,
            self.hot_keys,
            self.hot,
            self.gets,
            self.puts,
            self.deletes,
            self.value_len_max,
            self.think,
            self.think_max
        )
    }
}

/// What a client does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Request(Request),
    /// Nothing, for this long.
    Think(Duration),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkloadStats {
    pub requests: u64,
    pub gets: u64,
    pub puts: u64,
    pub deletes: u64,
    /// Requests that went to the hot set.
    pub hot: u64,
    pub thinks: u64,
}

/// The generator. Owns its dice, forked from the seed, so that adding a
/// draw elsewhere in the simulator does not change what the clients ask.
#[derive(Clone, Debug)]
pub struct Workload {
    config: WorkloadConfig,
    prng: Prng,
    stats: WorkloadStats,
}

impl Workload {
    pub fn new(config: WorkloadConfig, prng: Prng) -> Workload {
        config.validate();
        Workload {
            config,
            prng,
            stats: WorkloadStats::default(),
        }
    }

    pub fn config(&self) -> &WorkloadConfig {
        &self.config
    }

    pub fn stats(&self) -> WorkloadStats {
        self.stats
    }

    /// The next step for `client`, which is idle.
    pub fn next(&mut self, client: ClientId) -> Step {
        let c = &self.config;
        if c.think.roll(&mut self.prng) {
            self.stats.thinks += 1;
            let nanos = u64::try_from(c.think_max.as_nanos()).expect("think time fits");
            return Step::Think(Duration::from_nanos(self.prng.range(1, nanos + 1)));
        }
        let hot = c.hot.roll(&mut self.prng);
        let index = if hot {
            self.prng.below(u64::from(c.hot_keys))
        } else {
            self.prng.below(u64::from(c.keys))
        } as u32;
        let key = WorkloadConfig::key(index);
        self.stats.requests += 1;
        self.stats.hot += u64::from(hot || index < c.hot_keys);
        let roll = self.prng.below(u64::from(c.gets + c.puts + c.deletes)) as u32;
        let request = if roll < c.gets {
            self.stats.gets += 1;
            Request::Get { key }
        } else if roll < c.gets + c.puts {
            self.stats.puts += 1;
            let value = self.value(client, self.stats.requests);
            Request::Put { key, value }
        } else {
            self.stats.deletes += 1;
            Request::Delete { key }
        };
        Step::Request(request)
    }

    /// A value that names its writer and its place in the run, padded to
    /// a random length. No two writes in a run carry the same value.
    fn value(&mut self, client: ClientId, n: u64) -> Vec<u8> {
        let mut value: String = format!("c{client}-{n}");
        let len = self.prng.range(1, u64::from(self.config.value_len_max) + 1) as usize;
        while value.len() < len {
            value.push('.');
        }
        value.into_bytes()
    }
}
