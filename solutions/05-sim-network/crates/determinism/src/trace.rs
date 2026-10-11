//! An append-only event log with a running hash.
//!
//! The trace is how we define "the same run". Every decision a simulation
//! makes is recorded as one line; the hash of all lines is the run's
//! fingerprint. Two runs with the same seed must produce the same hash, bit
//! for bit. When they do not, the stored lines let us print the first point
//! of divergence instead of just "hashes differ".
//!
//! The hash is FNV-1a over the UTF-8 bytes of each line plus a newline. It is
//! not cryptographic and does not need to be. What it needs to be is
//! *stable*: implemented here, never from a crate whose algorithm could
//! change under us.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

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
        Trace {
            lines: Vec::new(),
            hash: FNV_OFFSET,
            keep_lines: true,
        }
    }

    /// A trace that only keeps the hash. Use for long runs where storing
    /// millions of lines would dominate memory.
    pub fn hash_only() -> Trace {
        Trace {
            lines: Vec::new(),
            hash: FNV_OFFSET,
            keep_lines: false,
        }
    }

    /// Record one event. Prefer the [`trace!`](crate::trace!) macro.
    pub fn record(&mut self, args: fmt::Arguments<'_>) {
        let line = args.to_string();
        self.hash_bytes(line.as_bytes());
        self.hash_bytes(b"\n");
        if self.keep_lines {
            self.lines.push(line);
        }
    }

    fn hash_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= u64::from(b);
            self.hash = self.hash.wrapping_mul(FNV_PRIME);
        }
    }

    /// The fingerprint of everything recorded so far.
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Number of events recorded.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// `true` if nothing has been recorded.
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
        let n = self.lines.len().max(other.lines.len());
        (0..n).find_map(|i| {
            let a = self.lines.get(i).map(String::as_str);
            let b = other.lines.get(i).map(String::as_str);
            (a != b).then_some((i, a, b))
        })
    }
}

/// Record a formatted event into a [`Trace`].
///
/// ```
/// # use determinism::{trace, Trace};
/// let mut t = Trace::new();
/// trace!(t, "node {} sent {} bytes", 1, 32);
/// assert_eq!(t.lines()[0], "node 1 sent 32 bytes");
/// ```
#[macro_export]
macro_rules! trace {
    ($trace:expr, $($arg:tt)*) => {
        $trace.record(format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_across_instances() {
        let mut a = Trace::new();
        let mut b = Trace::hash_only();
        for i in 0..100 {
            trace!(a, "event {i}");
            trace!(b, "event {i}");
        }
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.len(), 100);
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn hash_known_answer() {
        // Pins the hash algorithm. If this ever changes, every recorded seed
        // fingerprint in the course is invalidated, so change it knowingly.
        let mut t = Trace::new();
        trace!(t, "hello");
        assert_eq!(t.hash(), fnv1a(b"hello\n"));
        assert_eq!(fnv1a(b""), FNV_OFFSET);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes
            .iter()
            .fold(FNV_OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(FNV_PRIME))
    }

    #[test]
    fn line_boundaries_matter() {
        let mut a = Trace::new();
        trace!(a, "ab");
        trace!(a, "c");
        let mut b = Trace::new();
        trace!(b, "a");
        trace!(b, "bc");
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn reports_first_divergence() {
        let mut a = Trace::new();
        let mut b = Trace::new();
        trace!(a, "same");
        trace!(b, "same");
        trace!(a, "x");
        trace!(b, "y");
        assert_eq!(a.first_divergence(&b), Some((1, Some("x"), Some("y"))));
        trace!(a, "tail");
        assert_eq!(a.first_divergence(&b), Some((1, Some("x"), Some("y"))));
        let mut c = a.clone();
        assert_eq!(a.first_divergence(&c), None);
        trace!(c, "extra");
        assert_eq!(a.first_divergence(&c), Some((3, None, Some("extra"))));
    }
}
