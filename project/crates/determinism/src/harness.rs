//! The determinism check: run under a seed, run again, compare.

use crate::{Prng, Trace};
use std::fmt;
use std::path::Path;

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
        write!(
            f,
            "seed {} is not deterministic: trace hashes {:#018x} vs {:#018x}",
            self.seed, self.first_hash, self.second_hash
        )
        // Module 1: also print the first differing line from each run.
    }
}

impl std::error::Error for Divergence {}

/// Run `program` twice under `seed` and compare the traces.
pub fn run_twice<F>(seed: u64, program: F) -> Result<u64, Divergence>
where
    F: FnMut(&mut Prng, &mut Trace),
{
    check_determinism(seed, 2, program)
}

/// Run `program` `runs` times under `seed`, comparing each run to the first.
pub fn check_determinism<F>(seed: u64, runs: usize, program: F) -> Result<u64, Divergence>
where
    F: FnMut(&mut Prng, &mut Trace),
{
    let _ = (seed, runs, program);
    todo!("Module 1: fresh Prng and Trace per run; compare hashes; build a Divergence")
}

/// Why a cross-process check could not produce a verdict.
#[derive(Debug)]
pub enum ProcessCheckError {
    Spawn(std::io::Error),
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    NoHash(String),
    Diverged(Divergence),
}

impl fmt::Display for ProcessCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ProcessCheckError {}

/// The line a program prints so that [`run_process_twice`] can read its hash.
pub fn format_hash_line(hash: u64) -> String {
    format!("trace_hash={hash:016x}")
}

/// Run `program` twice as a child process with `--seed <seed>` appended to
/// `args`, and compare the `trace_hash=` lines each prints on stdout.
pub fn run_process_twice(program: &Path, args: &[&str], seed: u64) -> Result<u64, ProcessCheckError> {
    let _ = (program, args, seed);
    todo!("Module 1: std::process::Command, parse the last `trace_hash=` line")
}

/// Parse `--seed N` (or `--seed=N`) from arguments, defaulting to `default`.
pub fn seed_from_args(args: impl IntoIterator<Item = String>, default: u64) -> u64 {
    let _ = (args, default);
    todo!("Module 1")
}
