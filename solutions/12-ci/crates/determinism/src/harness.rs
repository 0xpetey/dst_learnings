//! The determinism check.
//!
//! The idea is embarrassingly simple and that is the point: run the program
//! under a seed, record its trace, run it again under the same seed, and
//! compare. Anything that differs is nondeterminism, by definition. This is
//! the test that every later module keeps in its acceptance criteria.
//!
//! Two flavours:
//!
//! * [`run_twice`] / [`check_determinism`] run a closure in-process. Fast,
//!   and catches most hazards (clocks, OS randomness, hash-map order, thread
//!   interleavings).
//! * [`run_process_twice`] spawns a binary twice as separate OS processes and
//!   compares the hash each prints. Slower, but it is the only way to catch
//!   hazards that are fixed for the lifetime of a process: memory addresses
//!   under ASLR, a `static` initialised once, an environment variable read
//!   at startup.

use crate::{Prng, Trace};
use std::fmt;
use std::path::Path;
use std::process::Command;

/// The first point where two runs under the same seed disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    pub seed: u64,
    /// Index of the first differing trace line, if lines were kept.
    pub index: Option<usize>,
    pub first: Option<String>,
    pub second: Option<String>,
    pub first_hash: u64,
    pub second_hash: u64,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "seed {} is not deterministic: trace hashes {:#018x} vs {:#018x}",
            self.seed, self.first_hash, self.second_hash
        )?;
        match self.index {
            Some(i) => {
                writeln!(f, "first divergence at line {i}:")?;
                writeln!(
                    f,
                    "  run 1: {}",
                    self.first.as_deref().unwrap_or("<end of trace>")
                )?;
                write!(
                    f,
                    "  run 2: {}",
                    self.second.as_deref().unwrap_or("<end of trace>")
                )
            }
            None => write!(f, "(trace lines not available)"),
        }
    }
}

impl std::error::Error for Divergence {}

/// Run `program` twice under `seed` and compare the traces.
///
/// Returns the common trace hash on success. The closure receives a fresh
/// `Prng` and `Trace` each time; everything else it touches must be freshly
/// constructed inside it, or it is a hazard.
pub fn run_twice<F>(seed: u64, mut program: F) -> Result<u64, Divergence>
where
    F: FnMut(&mut Prng, &mut Trace),
{
    let first = run_once(seed, &mut program);
    let second = run_once(seed, &mut program);
    compare(seed, &first, &second)
}

/// Run `program` `runs` times under `seed`, comparing each run against the
/// first. More runs catch hazards that only *sometimes* change the trace,
/// such as a thread interleaving that usually comes out the same.
pub fn check_determinism<F>(seed: u64, runs: usize, mut program: F) -> Result<u64, Divergence>
where
    F: FnMut(&mut Prng, &mut Trace),
{
    assert!(runs >= 2, "a determinism check needs at least two runs");
    let first = run_once(seed, &mut program);
    for _ in 1..runs {
        let next = run_once(seed, &mut program);
        compare(seed, &first, &next)?;
    }
    Ok(first.hash())
}

fn run_once<F>(seed: u64, program: &mut F) -> Trace
where
    F: FnMut(&mut Prng, &mut Trace),
{
    let mut prng = Prng::from_seed(seed);
    let mut trace = Trace::new();
    program(&mut prng, &mut trace);
    trace
}

fn compare(seed: u64, first: &Trace, second: &Trace) -> Result<u64, Divergence> {
    if first.hash() == second.hash() {
        return Ok(first.hash());
    }
    let (index, a, b) = match first.first_divergence(second) {
        Some((i, a, b)) => (Some(i), a.map(str::to_owned), b.map(str::to_owned)),
        None => (None, None, None),
    };
    Err(Divergence {
        seed,
        index,
        first: a,
        second: b,
        first_hash: first.hash(),
        second_hash: second.hash(),
    })
}

/// Why a cross-process check could not produce a verdict.
#[derive(Debug)]
pub enum ProcessCheckError {
    Spawn(std::io::Error),
    /// The program exited unsuccessfully; stderr attached.
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    /// The program's stdout did not end in a line `trace_hash=<hex>`.
    NoHash(String),
    Diverged(Divergence),
}

impl fmt::Display for ProcessCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessCheckError::Spawn(e) => write!(f, "could not spawn program: {e}"),
            ProcessCheckError::Failed { status, stderr } => {
                write!(f, "program failed with {status}: {stderr}")
            }
            ProcessCheckError::NoHash(out) => {
                write!(
                    f,
                    "program did not print a `trace_hash=` line; stdout was: {out:?}"
                )
            }
            ProcessCheckError::Diverged(d) => d.fmt(f),
        }
    }
}

impl std::error::Error for ProcessCheckError {}

/// The line a program prints so that [`run_process_twice`] can read its hash.
pub fn format_hash_line(hash: u64) -> String {
    format!("trace_hash={hash:016x}")
}

/// Run `program` twice as a child process with `--seed <seed>` appended to
/// `args`, and compare the `trace_hash=` lines each prints on stdout.
///
/// The program is responsible for building a `Trace`, running, and printing
/// [`format_hash_line`] as its last stdout line.
pub fn run_process_twice(program: &Path, args: &[&str], seed: u64) -> Result<u64, ProcessCheckError> {
    let first = run_process_once(program, args, seed)?;
    let second = run_process_once(program, args, seed)?;
    if first == second {
        Ok(first)
    } else {
        Err(ProcessCheckError::Diverged(Divergence {
            seed,
            index: None,
            first: None,
            second: None,
            first_hash: first,
            second_hash: second,
        }))
    }
}

fn run_process_once(program: &Path, args: &[&str], seed: u64) -> Result<u64, ProcessCheckError> {
    let seed_str = seed.to_string();
    let output = Command::new(program)
        .args(args)
        .arg("--seed")
        .arg(&seed_str)
        .output()
        .map_err(ProcessCheckError::Spawn)?;
    if !output.status.success() {
        return Err(ProcessCheckError::Failed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("trace_hash="))
        .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
        .ok_or_else(|| ProcessCheckError::NoHash(stdout.into_owned()))
}

/// Parse `--seed N` from a program's arguments, defaulting to `default`.
/// Shared by the example binaries so they all speak the same dialect.
pub fn seed_from_args(args: impl IntoIterator<Item = String>, default: u64) -> u64 {
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        if a == "--seed" {
            if let Some(v) = args.next() {
                return v.parse().expect("--seed takes an integer");
            }
        } else if let Some(v) = a.strip_prefix("--seed=") {
            return v.parse().expect("--seed takes an integer");
        }
    }
    default
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace;

    #[test]
    fn deterministic_program_passes() {
        let hash = run_twice(1, |prng, t| {
            for _ in 0..10 {
                trace!(t, "roll {}", prng.below(6));
            }
        })
        .expect("pure function of the seed");
        assert_eq!(
            check_determinism(1, 5, |prng, t| {
                for _ in 0..10 {
                    trace!(t, "roll {}", prng.below(6));
                }
            })
            .unwrap(),
            hash
        );
    }

    #[test]
    fn state_outside_the_closure_is_caught() {
        let mut calls = 0u32;
        let err = run_twice(1, |_prng, t| {
            calls += 1;
            trace!(t, "call number {calls}");
        })
        .unwrap_err();
        assert_eq!(err.index, Some(0));
        assert_eq!(err.first.as_deref(), Some("call number 1"));
        assert_eq!(err.second.as_deref(), Some("call number 2"));
        assert!(err.to_string().contains("seed 1 is not deterministic"));
    }

    #[test]
    fn seed_parsing() {
        let v = |s: &[&str]| seed_from_args(s.iter().map(|s| s.to_string()), 99);
        assert_eq!(v(&[]), 99);
        assert_eq!(v(&["--seed", "7"]), 7);
        assert_eq!(v(&["x", "--seed=8"]), 8);
    }
}
