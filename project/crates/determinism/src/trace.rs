//! An append-only event log with a running hash.
//!
//! The hash is FNV-1a 64-bit over the UTF-8 bytes of each line followed by
//! a newline. `tests` pin the known answer; keep the algorithm exactly that.

use std::fmt;

/// Event log plus running hash.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    lines: Vec<String>,
    hash: u64,
    keep_lines: bool,
}

impl Trace {
    /// A trace that stores every line (needed to report divergences).
    pub fn new() -> Trace {
        todo!("Module 1: start the hash at the FNV-1a offset basis")
    }

    /// A trace that only keeps the hash.
    pub fn hash_only() -> Trace {
        todo!("Module 1")
    }

    /// Record one event. Prefer the [`trace!`](crate::trace!) macro.
    pub fn record(&mut self, args: fmt::Arguments<'_>) {
        let _ = args;
        todo!("Module 1: format the line, hash it plus a newline, store it if keep_lines")
    }

    /// The fingerprint of everything recorded so far.
    pub fn hash(&self) -> u64 {
        self.hash
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The recorded lines (empty for a `hash_only` trace).
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Index and contents of the first line where two traces differ, if any.
    pub fn first_divergence<'a>(
        &'a self,
        other: &'a Trace,
    ) -> Option<(usize, Option<&'a str>, Option<&'a str>)> {
        let _ = other;
        todo!("Module 1")
    }
}

/// Record a formatted event into a [`Trace`].
#[macro_export]
macro_rules! trace {
    ($trace:expr, $($arg:tt)*) => {
        $trace.record(format_args!($($arg)*))
    };
}
