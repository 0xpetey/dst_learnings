//! The shrinker: a failing seed in, the smallest run that still fails out.
//!
//! A seed has no neighbours. Seed 563 says nothing about seed 562. So
//! the shrinker does not search seeds. It keeps the seed, which keeps
//! every die the run rolls, and turns the configuration down one knob at
//! a time: fewer requests, a fault family off, a rate inside one off,
//! three nodes for five, one client per node for two. A change is kept
//! on one condition: the run still fails *the same way*. Otherwise it is
//! undone and the next knob is tried. The loop runs until a whole pass
//! keeps nothing. That is the greedy heart of every shrinker from
//! QuickCheck to Hypothesis, and of Zeller's delta debugging.
//!
//! "The same way" is the [`Failure::signature`]: the first line of the
//! assertion with every number blanked. A shrink that turns a lost
//! record into a stale read has found a second bug, not a smaller first
//! one, and the shrinker says so and moves on.
//!
//! The runs happen through an [`Oracle`], because the workspace aborts
//! on a panic. The binary's oracle is a child process. The tests' oracle
//! is `catch_unwind`, which the test profile allows.

use crate::overrides::{reproduction, Knob, Overrides, DEFAULT_REQUESTS};
use crate::RunConfig;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::Command;

/// What a run that failed said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The assertion's message, every line of it.
    pub message: String,
}

impl Failure {
    pub fn first_line(&self) -> &str {
        self.message.lines().next().unwrap_or("").trim()
    }

    /// The first line, cut at the first bracket, with every number
    /// blanked. "told to truncate record 27, committed through 27" and
    /// "... record 112, committed through 112" are the same bug, found
    /// at different lengths. The checker's list of operations is cut
    /// off: the bug is "key k3 is not linearizable", whatever the
    /// history that showed it.
    pub fn signature(&self) -> String {
        let line = self.first_line();
        let end = line.find(['[', '(']).unwrap_or(line.len());
        let mut sig = String::new();
        let mut in_number = false;
        for c in line[..end].chars() {
            if c.is_ascii_digit() {
                if !in_number {
                    sig.push('#');
                }
                in_number = true;
            } else {
                in_number = false;
                sig.push(c);
            }
        }
        sig.trim().to_string()
    }
}

/// Runs a seed under overrides and reports the failure, if any.
pub trait Oracle {
    fn run(&mut self, seed: u64, overrides: &Overrides) -> Option<Failure>;
}

/// An oracle that calls [`crate::run`] in this process and catches the
/// panic. Only for a test profile, which keeps `panic = "unwind"`. In
/// the release binary the first failure ends the process.
pub struct InProcess;

impl Oracle for InProcess {
    fn run(&mut self, seed: u64, overrides: &Overrides) -> Option<Failure> {
        let config = overrides.configure(seed);
        run_catching(&config)
    }
}

/// Run `config` and return the message of the check that fired, if one
/// did. The panic hook is silenced for the run, so a shrink of thirty
/// candidates does not print thirty backtraces.
pub fn run_catching(config: &RunConfig) -> Option<Failure> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = catch_unwind(AssertUnwindSafe(|| crate::run(config)));
    std::panic::set_hook(hook);
    let err = outcome.err()?;
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    Some(Failure { message })
}

/// An oracle that runs `vopr --seed N <overrides>` as a child process.
/// A failure is a non-zero exit, and the message is what the panic hook
/// wrote to stderr between `panicked at` and the backtrace note.
pub struct ChildProcess {
    pub binary: PathBuf,
    /// Child processes spawned so far.
    pub runs: usize,
}

impl ChildProcess {
    pub fn new(binary: PathBuf) -> ChildProcess {
        ChildProcess { binary, runs: 0 }
    }

    /// The running binary, for `vopr --shrink` to call itself.
    pub fn of_this_binary() -> ChildProcess {
        ChildProcess::new(std::env::current_exe().expect("the path of this binary"))
    }
}

impl Oracle for ChildProcess {
    fn run(&mut self, seed: u64, overrides: &Overrides) -> Option<Failure> {
        self.runs += 1;
        let output = Command::new(&self.binary)
            .arg("--seed")
            .arg(seed.to_string())
            .args(overrides.to_args())
            .output()
            .unwrap_or_else(|e| panic!("could not run {}: {e}", self.binary.display()));
        if output.status.success() {
            return None;
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Some(Failure {
            message: panic_message(&stderr),
        })
    }
}

/// The panic message out of a child's stderr: the lines after
/// `panicked at <file>:<line>:<col>:` and before the backtrace note.
/// Anything else means the child died without a panic, and the whole
/// stderr is the message.
pub fn panic_message(stderr: &str) -> String {
    let mut lines = stderr.lines();
    if lines.by_ref().find(|l| l.contains("panicked at")).is_none() {
        let tail = stderr.trim();
        return if tail.is_empty() {
            "exited without a message".to_string()
        } else {
            tail.to_string()
        };
    }
    let message: Vec<&str> = lines
        .take_while(|l| !l.starts_with("note: run with") && !l.starts_with("stack backtrace"))
        .collect();
    message.join("\n").trim().to_string()
}

/// One candidate the shrinker tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    /// What was changed, as the flag or the number.
    pub change: String,
    pub overrides: Overrides,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The same failure. The change is kept.
    Kept,
    /// No failure. The change is undone.
    Passed,
    /// A different failure, with its signature. Undone, and noted: it
    /// is a second bug, or the same bug seen from another check.
    Other(String),
}

/// What the shrinker ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shrunk {
    pub seed: u64,
    pub start: Overrides,
    pub result: Overrides,
    pub signature: String,
    /// The failure under `result`.
    pub failure: Failure,
    /// Every candidate, in order.
    pub attempts: Vec<Attempt>,
    /// Signatures of other failures met on the way, each once.
    pub others: Vec<String>,
    /// Runs made, counting the first and the two that confirmed the end.
    pub runs: usize,
}

impl Shrunk {
    pub fn kept(&self) -> usize {
        self.attempts
            .iter()
            .filter(|a| a.outcome == Outcome::Kept)
            .count()
    }
}

impl fmt::Display for Shrunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "shrunk seed {} in {} runs: {} of {} changes kept",
            self.seed,
            self.runs,
            self.kept(),
            self.attempts.len()
        )?;
        writeln!(f, "  failure: {}", self.failure.first_line())?;
        for other in &self.others {
            writeln!(f, "  also seen: {other}")?;
        }
        writeln!(f, "  from: {}", reproduction(self.seed, &self.start))?;
        write!(f, "  to:   {}", reproduction(self.seed, &self.result))
    }
}

/// Why a shrink could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShrinkError {
    /// The seed passed under the starting overrides. Nothing to shrink.
    Passes,
    /// The seed failed, then passed or failed differently on the same
    /// run again. That is nondeterminism, and the Module 1 check is the
    /// tool for it, not the shrinker.
    Unstable { first: String, second: String },
}

impl fmt::Display for ShrinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShrinkError::Passes => write!(f, "the seed passes: nothing to shrink"),
            ShrinkError::Unstable { first, second } => write!(
                f,
                "the run is not deterministic: it failed with {first:?} and then with {second:?}"
            ),
        }
    }
}

impl std::error::Error for ShrinkError {}

/// The shrinker's state between candidates.
struct Search<'a> {
    seed: u64,
    signature: String,
    oracle: &'a mut dyn Oracle,
    log: &'a mut dyn FnMut(&Attempt),
    current: Overrides,
    attempts: Vec<Attempt>,
    others: Vec<String>,
    runs: usize,
}

impl Search<'_> {
    /// Try one candidate. Keep it if the failure is the same.
    fn attempt(&mut self, change: String, candidate: Overrides) -> bool {
        self.runs += 1;
        let outcome = match self.oracle.run(self.seed, &candidate) {
            None => Outcome::Passed,
            Some(f) if f.signature() == self.signature => Outcome::Kept,
            Some(f) => {
                let sig = f.signature();
                if !self.others.contains(&sig) {
                    self.others.push(sig.clone());
                }
                Outcome::Other(sig)
            }
        };
        let kept = outcome == Outcome::Kept;
        let a = Attempt {
            change,
            overrides: candidate.clone(),
            outcome,
        };
        (self.log)(&a);
        self.attempts.push(a);
        if kept {
            self.current = candidate;
        }
        kept
    }

    /// One pass: the requests, then every knob. Returns whether a knob
    /// was kept. A kept knob changes what the run does, so the request
    /// search before it is stale and another pass is due. A pass that
    /// kept no knob has searched the requests for the final run.
    fn pass(&mut self) -> bool {
        // 1. Requests. `lo` passes (or is zero), `hi` fails.
        let mut hi = self.current.requests.unwrap_or(DEFAULT_REQUESTS);
        let mut lo = 0;
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            let mut candidate = self.current.clone();
            candidate.requests = Some(mid);
            if self.attempt(format!("--requests {mid}"), candidate) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let after_requests = self.current.clone();

        // 2. Knobs, coarse to fine.
        for knob in Knob::ALL {
            if self.current.off.contains(&knob) || !knob.is_live(&self.current.configure(self.seed)) {
                continue;
            }
            let mut candidate = self.current.clone();
            candidate.off.insert(knob);
            self.attempt(knob.flag().to_string(), candidate);
        }

        // 3. The cluster.
        let config = self.current.configure(self.seed);
        if config.nodes > 3 {
            let mut candidate = self.current.clone();
            candidate.nodes = Some(3);
            self.attempt("--nodes 3".to_string(), candidate);
        }
        if config.clients_per_node > 1 {
            let mut candidate = self.current.clone();
            candidate.clients_per_node = Some(1);
            self.attempt("--clients 1".to_string(), candidate);
        }

        self.current != after_requests
    }
}

/// Shrink `seed` from `start`. `log` sees every attempt as it is made.
///
/// Each pass, in order:
///
/// 1. Requests, by binary search between zero and the current count.
///    Shorter runs make every later candidate cheaper.
/// 2. Each live knob, coarse to fine: the families, then the rates
///    inside them, then the core.
/// 3. Five nodes to three, two clients per node to one.
///
/// Passes repeat until one keeps nothing. The result is checked twice
/// more at the end. A reproduction that fails only sometimes is no
/// reproduction.
pub fn shrink(
    seed: u64,
    start: &Overrides,
    oracle: &mut dyn Oracle,
    mut log: impl FnMut(&Attempt),
) -> Result<Shrunk, ShrinkError> {
    let Some(failure) = oracle.run(seed, start) else {
        return Err(ShrinkError::Passes);
    };
    let signature = failure.signature();
    let mut search = Search {
        seed,
        signature: signature.clone(),
        oracle,
        log: &mut log,
        current: start.clone(),
        attempts: Vec::new(),
        others: Vec::new(),
        runs: 1,
    };
    while search.pass() {}

    // The reproduction, twice. Module 1's check, on the thing that matters.
    let mut last = None;
    for _ in 0..2 {
        search.runs += 1;
        let again = search.oracle.run(seed, &search.current);
        let sig = again
            .as_ref()
            .map(Failure::signature)
            .unwrap_or_else(|| "pass".into());
        if sig != signature {
            return Err(ShrinkError::Unstable {
                first: signature,
                second: sig,
            });
        }
        last = again;
    }
    Ok(Shrunk {
        seed,
        start: start.clone(),
        result: search.current,
        signature,
        failure: last.expect("failed twice"),
        attempts: search.attempts,
        others: search.others,
        runs: search.runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_blanks_numbers() {
        let f = Failure {
            message: "node 1 view 71: told to truncate record 27, committed through 27\nmore".into(),
        };
        assert_eq!(
            f.signature(),
            "node # view #: told to truncate record #, committed through #"
        );
        assert_eq!(
            f.first_line(),
            "node 1 view 71: told to truncate record 27, committed through 27"
        );
        let f = Failure {
            message: "safety: key \"k3\" is not linearizable: not linearizable: after 9 operations [0, 1, 2], none of these fit: op 9 (model says Value(None))".into(),
        };
        assert_eq!(
            f.signature(),
            "safety: key \"k#\" is not linearizable: not linearizable: after # operations"
        );
    }

    #[test]
    fn panic_message_is_cut_out_of_stderr() {
        let stderr = "\nthread 'main' panicked at crates/kv-core/src/replica.rs:1127:9:\nnode 1 view 71: told to truncate record 27, committed through 27\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n";
        assert_eq!(
            panic_message(stderr),
            "node 1 view 71: told to truncate record 27, committed through 27"
        );
        let multi = "thread 'main' panicked at x.rs:1:1:\nsafety: key \"k1\" is not linearizable: ...\n    0 c1 id=1\n    1 c2 id=1\nnote: run with\n";
        assert_eq!(
            panic_message(multi),
            "safety: key \"k1\" is not linearizable: ...\n    0 c1 id=1\n    1 c2 id=1"
        );
        assert_eq!(panic_message("Killed\n"), "Killed");
        assert_eq!(panic_message(""), "exited without a message");
    }

    /// A pretend oracle: fails with one signature while `requests >= 40`
    /// and crashes are on, with another once the disk is reliable, and
    /// passes otherwise.
    struct Pretend {
        runs: usize,
    }

    impl Oracle for Pretend {
        fn run(&mut self, seed: u64, o: &Overrides) -> Option<Failure> {
            self.runs += 1;
            let c = o.configure(seed);
            if c.requests < 40 || c.world.crashes.is_none() {
                return None;
            }
            if c.world.disk.is_reliable() {
                return Some(Failure {
                    message: "liveness: stuck at 12".into(),
                });
            }
            Some(Failure {
                message: format!("safety: lost record {}", c.requests),
            })
        }
    }

    #[test]
    fn greedy_shrink_keeps_what_still_fails() {
        // A seed whose draw has crashes, a faulty disk and pauses.
        let seed = (0..500)
            .find(|&s| {
                let c = RunConfig::from_seed(s, 150);
                c.world.crashes.is_some() && !c.world.disk.is_reliable() && c.world.pauses.is_some()
            })
            .unwrap();
        let mut oracle = Pretend { runs: 0 };
        let shrunk = shrink(seed, &Overrides::default(), &mut oracle, |_| {}).unwrap();
        assert_eq!(shrunk.result.requests, Some(40));
        assert!(!shrunk.result.off.contains(&Knob::Crashes));
        assert!(!shrunk.result.off.contains(&Knob::Disk));
        assert!(shrunk.result.off.contains(&Knob::Pauses));
        assert_eq!(shrunk.signature, "safety: lost record #");
        assert_eq!(shrunk.others, vec!["liveness: stuck at #".to_string()]);
        assert_eq!(shrunk.runs, oracle.runs);
        let kept: Vec<&str> = shrunk
            .attempts
            .iter()
            .filter(|a| a.outcome == Outcome::Kept)
            .map(|a| a.change.as_str())
            .collect();
        assert!(kept.contains(&"--no-pauses"), "{kept:?}");
        assert!(shrunk.to_string().contains("--requests 40"));
    }

    #[test]
    fn a_passing_seed_is_not_shrunk() {
        struct Passes;
        impl Oracle for Passes {
            fn run(&mut self, _: u64, _: &Overrides) -> Option<Failure> {
                None
            }
        }
        assert_eq!(
            shrink(1, &Overrides::default(), &mut Passes, |_| {}),
            Err(ShrinkError::Passes)
        );
    }

    #[test]
    fn an_unstable_failure_is_refused() {
        struct Flaky(u32);
        impl Oracle for Flaky {
            fn run(&mut self, _: u64, _: &Overrides) -> Option<Failure> {
                self.0 += 1;
                (self.0 == 1).then(|| Failure {
                    message: "safety: once".into(),
                })
            }
        }
        let err = shrink(1, &Overrides::default(), &mut Flaky(0), |_| {}).unwrap_err();
        assert!(matches!(err, ShrinkError::Unstable { .. }), "{err}");
    }
}
