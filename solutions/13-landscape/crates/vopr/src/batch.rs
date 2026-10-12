//! The batch: many seeds, the way CI runs them.
//!
//! ```text
//!   commit ──► seeds_for_commit ──► seeds, `jobs` at a time, in child processes
//!                                        │
//!                      passed ──► cost against the ceiling, hash into the sweep
//!                      failed ──► run twice more with the trace, compare
//!                                        │
//!                              stable ──► Record (+ shrink, if asked)
//!                              unstable ► Record that says so
//!                                        │
//!                                   BatchReport, and one file per failure
//! ```
//!
//! Three rules make a batch worth running unattended.
//!
//! 1. The seeds follow the commit. The same commit runs the same seeds,
//!    so a verdict is reproducible, and the next commit runs others, so
//!    the pushes add up to coverage.
//! 2. A failure is never retried into a pass. A failing seed is run
//!    twice more with its trace, and the three runs have to agree. When
//!    they do, the failure is real and the record says how to reproduce
//!    it. When they do not, the binary is not deterministic, which is a
//!    bug of its own, and the record says that instead.
//! 3. The budget is in the seed's units. A run that fires more scheduler
//!    events per request than the ceiling allows is a regression on every
//!    machine, loaded or not. Wall-clock time is reported, never asserted.
//!
//! The runs happen through a [`Runner`], as the shrinker's do through an
//! [`Oracle`]: child processes in the binary, `catch_unwind` in the tests.

use crate::overrides::{reproduction, Overrides};
use crate::shrink::{self, panic_message, run_catching, Failure, Oracle};
use crate::RunConfig;
use determinism::Trace;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Hex digits of the commit hash that name the batch's first seed.
const COMMIT_DIGITS: usize = 12;

/// The one wall clock in the simulator binary. It bounds the machine's
/// time and reports it. No run reads it, and nothing asserts on it.
#[expect(
    clippy::disallowed_types,
    reason = "the batch's budget is wall-clock on purpose"
)]
pub struct Stopwatch {
    start: std::time::Instant,
}

impl Stopwatch {
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "the batch's budget is wall-clock on purpose"
    )]
    pub fn start() -> Stopwatch {
        Stopwatch {
            start: std::time::Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// The seeds a commit gets: `count` of them, starting at the number the
/// first twelve hex digits of the hash spell. Twelve digits are
/// forty-eight bits, so the start is below `2^48` and the range cannot
/// overflow.
pub fn seeds_for_commit(commit: &str, count: u64) -> Result<Range<u64>, String> {
    let hex = commit.trim();
    if hex.len() < COMMIT_DIGITS || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "not a commit hash: {commit:?} (twelve or more hex digits)"
        ));
    }
    let start = u64::from_str_radix(&hex[..COMMIT_DIGITS], 16).map_err(|e| e.to_string())?;
    Ok(start..start + count)
}

/// What one run cost, in units the seed decides and the machine does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// Scheduler events fired.
    pub events: u64,
    /// Requests in the chaos phase.
    pub requests: u64,
    /// Steps of the linearizability search.
    pub check_steps: u64,
}

impl Cost {
    pub fn events_per_request(&self) -> u64 {
        self.events / self.requests.max(1)
    }
}

/// The ceilings a run has to stay under. A run over one is a performance
/// regression, and it is found the same way on every machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ceiling {
    pub events_per_request: u64,
    pub check_steps: u64,
}

impl Default for Ceiling {
    /// About twice the most any seed of twenty thousand needed when the
    /// module was written: 427 events per request at seed 16471, and
    /// 358 481 check steps at seed 4888. The lesson has the distribution.
    fn default() -> Ceiling {
        Ceiling {
            events_per_request: 1_000,
            check_steps: 800_000,
        }
    }
}

impl Ceiling {
    /// What `cost` exceeds, if anything.
    pub fn exceeded(&self, cost: &Cost) -> Option<String> {
        if cost.events_per_request() > self.events_per_request {
            return Some(format!(
                "events per request {} over the ceiling of {}",
                cost.events_per_request(),
                self.events_per_request
            ));
        }
        if cost.check_steps > self.check_steps {
            return Some(format!(
                "check steps {} over the ceiling of {}",
                cost.check_steps, self.check_steps
            ));
        }
        None
    }
}

/// One run's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Passed { hash: u64, cost: Cost },
    Failed(Failure),
}

/// One run, as the batch saw it.
#[derive(Clone, Debug)]
pub struct Observed {
    pub seed: u64,
    pub verdict: Verdict,
    /// The hash of everything the run printed. For a run that aborted
    /// with its trace echoed, that is the trace up to the assertion, and
    /// two such runs have to agree on it.
    pub output_hash: u64,
    /// What the run printed, when its trace was asked for.
    pub output: Option<String>,
    /// Wall-clock time. Reported, never asserted.
    pub wall: Duration,
}

/// Runs seeds, several at a time where it can.
pub trait Runner {
    /// Run every seed under `overrides`, in order. With `trace`, each run
    /// echoes its trace and `output` carries it.
    fn run_many(&mut self, seeds: &[u64], overrides: &Overrides, trace: bool) -> Vec<Observed>;
}

/// The hash of `text`, line by line, as a trace would hash it.
pub fn hash_lines(text: &str) -> u64 {
    let mut t = Trace::hash_only();
    for line in text.lines() {
        t.record(format_args!("{line}"));
    }
    t.hash()
}

/// The number after `key=` on `line`, if there is one.
fn field(line: &str, key: &str) -> Option<u64> {
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix(key))
        .and_then(|v| v.trim_start_matches('=').parse().ok())
}

/// A child's stdout and stderr, read back as a verdict. A pass ends in
/// `trace_hash=` and carries a `cost:` line. A failure is a non-zero
/// exit, and its message is cut out of stderr as the shrinker cuts it.
pub fn observe(seed: u64, success: bool, stdout: &str, stderr: &str, wall: Duration) -> Observed {
    let output_hash = hash_lines(stdout);
    let verdict = if success {
        let hash = stdout
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("trace_hash="))
            .and_then(|h| u64::from_str_radix(h.trim(), 16).ok())
            .unwrap_or(0);
        let mut cost = Cost::default();
        for line in stdout.lines() {
            if line.starts_with("cost:") {
                cost.events = field(line, "events=").unwrap_or(0);
                cost.requests = field(line, "requests=").unwrap_or(0);
            } else if line.starts_with("check:") {
                cost.check_steps = field(line, "steps=").unwrap_or(0);
            }
        }
        Verdict::Passed { hash, cost }
    } else {
        Verdict::Failed(Failure {
            message: panic_message(stderr),
        })
    };
    Observed {
        seed,
        verdict,
        output_hash,
        output: None,
        wall,
    }
}

/// A runner that spawns `vopr --seed N <overrides>` as child processes,
/// `jobs` of them at a time. The results come back in seed order.
pub struct Children {
    pub binary: PathBuf,
    pub jobs: usize,
    /// Child processes spawned so far.
    pub runs: usize,
}

impl Children {
    pub fn new(binary: PathBuf, jobs: usize) -> Children {
        Children {
            binary,
            jobs: jobs.max(1),
            runs: 0,
        }
    }

    /// The running binary, for `vopr --batch` to call itself.
    pub fn of_this_binary(jobs: usize) -> Children {
        Children::new(std::env::current_exe().expect("the path of this binary"), jobs)
    }
}

impl Runner for Children {
    fn run_many(&mut self, seeds: &[u64], overrides: &Overrides, trace: bool) -> Vec<Observed> {
        let mut observed = Vec::with_capacity(seeds.len());
        let mut running = VecDeque::new();
        let mut next = seeds.iter();
        loop {
            while running.len() < self.jobs {
                let Some(&seed) = next.next() else { break };
                let mut cmd = Command::new(&self.binary);
                cmd.arg("--seed").arg(seed.to_string());
                if trace {
                    cmd.arg("--trace");
                }
                let child = cmd
                    .args(overrides.to_args())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap_or_else(|e| panic!("could not run {}: {e}", self.binary.display()));
                self.runs += 1;
                running.push_back((seed, child, Stopwatch::start()));
            }
            let Some((seed, child, started)) = running.pop_front() else {
                break;
            };
            let output = child.wait_with_output().expect("the child's output");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut o = observe(seed, output.status.success(), &stdout, &stderr, started.elapsed());
            if trace {
                o.output = Some(stdout.into_owned());
            }
            observed.push(o);
        }
        observed
    }
}

/// A runner that calls [`crate::run`] in this process and catches the
/// panic. For the test profile, which unwinds. A failure's output is
/// its message, because the world is gone by the time the panic is
/// caught, and its trace with it.
pub struct InProcess;

impl Runner for InProcess {
    fn run_many(&mut self, seeds: &[u64], overrides: &Overrides, trace: bool) -> Vec<Observed> {
        seeds
            .iter()
            .map(|&seed| {
                let config: RunConfig = overrides.configure(seed);
                let (verdict, output) = match run_catching(&config) {
                    Some(failure) => {
                        let text = format!("{config}\n{}", failure.message);
                        (Verdict::Failed(failure), text)
                    }
                    None => {
                        let report = crate::run(&config);
                        let cost = Cost {
                            events: report.events,
                            requests: config.requests,
                            check_steps: report.check.steps,
                        };
                        let text = format!("{config}\n{report}");
                        (
                            Verdict::Passed {
                                hash: report.hash,
                                cost,
                            },
                            text,
                        )
                    }
                };
                Observed {
                    seed,
                    verdict,
                    output_hash: hash_lines(&output),
                    output: trace.then_some(output),
                    wall: Duration::ZERO,
                }
            })
            .collect()
    }
}

/// Any runner is an oracle for the shrinker: one seed at a time, no trace.
struct AsOracle<'a>(&'a mut dyn Runner);

impl Oracle for AsOracle<'_> {
    fn run(&mut self, seed: u64, overrides: &Overrides) -> Option<Failure> {
        let mut observed = self.0.run_many(&[seed], overrides, false);
        match observed.pop().map(|o| o.verdict) {
            Some(Verdict::Failed(f)) => Some(f),
            _ => None,
        }
    }
}

/// What a batch is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub seeds: Range<u64>,
    pub overrides: Overrides,
    /// The commit the binary was built from, for the records.
    pub commit: Option<String>,
    /// Stop starting seeds once this much wall time has passed.
    pub budget: Option<Duration>,
    pub ceiling: Ceiling,
    /// Shrink every stable failure.
    pub shrink: bool,
    /// Seeds handed to the runner at once.
    pub jobs: usize,
}

impl Batch {
    pub fn new(seeds: Range<u64>, overrides: Overrides) -> Batch {
        Batch {
            seeds,
            overrides,
            commit: None,
            budget: None,
            ceiling: Ceiling::default(),
            shrink: false,
            jobs: 1,
        }
    }
}

/// One failing seed: the bug report, complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub seed: u64,
    pub commit: Option<String>,
    pub overrides: Overrides,
    pub signature: String,
    /// The hash of everything the run printed up to the assertion, the
    /// same twice over. `None` when the reruns did not agree.
    pub trace_hash: Option<u64>,
    /// What a rerun said instead, when one disagreed: `pass`, or another
    /// signature. A disagreement is nondeterminism, and it is reported,
    /// not retried.
    pub disagreement: Option<String>,
    /// The smallest run the shrinker found, when it was asked.
    pub shrunk: Option<Overrides>,
    /// The assertion's message, every line of it.
    pub message: String,
}

impl Record {
    pub fn stable(&self) -> bool {
        self.disagreement.is_none()
    }

    /// The command line that reproduces the failure as it was found.
    pub fn reproduction(&self) -> String {
        reproduction(self.seed, &self.overrides)
    }

    /// The file this record is written as. Key-value lines, then the
    /// message after `failure:`, indented. [`Record::parse`] reads it
    /// back.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("seed={}\n", self.seed));
        s.push_str(&format!("commit={}\n", self.commit.as_deref().unwrap_or("none")));
        s.push_str(&format!("overrides={}\n", self.overrides));
        s.push_str(&format!("signature={}\n", self.signature));
        match self.trace_hash {
            Some(h) => s.push_str(&format!("trace_hash={h:016x}\n")),
            None => s.push_str("trace_hash=none\n"),
        }
        s.push_str(&format!(
            "disagreement={}\n",
            self.disagreement.as_deref().unwrap_or("none")
        ));
        s.push_str(&format!("reproduction={}\n", self.reproduction()));
        if let Some(shrunk) = &self.shrunk {
            s.push_str(&format!("shrunk={}\n", reproduction(self.seed, shrunk)));
        }
        s.push_str("failure:\n");
        for line in self.message.lines() {
            s.push_str("  ");
            s.push_str(line);
            s.push('\n');
        }
        s
    }

    pub fn parse(text: &str) -> Result<Record, String> {
        let mut fields = BTreeMap::new();
        let mut lines = text.lines();
        for line in lines.by_ref() {
            if line == "failure:" {
                break;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| format!("not a field: {line:?}"))?;
            fields.insert(k.to_string(), v.to_string());
        }
        let message: Vec<&str> = lines.map(|l| l.strip_prefix("  ").unwrap_or(l)).collect();
        let get = |k: &str| fields.get(k).ok_or_else(|| format!("missing {k}"));
        let none = |v: &String| (v != "none").then(|| v.clone());
        let overrides_of = |line: &str| -> Result<Overrides, String> {
            let args: Vec<String> = line.split_whitespace().map(str::to_string).collect();
            let (o, rest) = Overrides::parse(&args)?;
            if !rest.is_empty() {
                return Err(format!("not override flags: {rest:?}"));
            }
            Ok(o)
        };
        let shrunk = match fields.get("shrunk") {
            None => None,
            Some(line) => {
                let flags = line
                    .split_once(" -- --seed ")
                    .and_then(|(_, rest)| rest.split_once(' ').map(|(_, f)| f))
                    .unwrap_or("");
                Some(overrides_of(flags)?)
            }
        };
        Ok(Record {
            seed: get("seed")?
                .parse()
                .map_err(|_| "seed: not a number".to_string())?,
            commit: none(get("commit")?),
            overrides: overrides_of(get("overrides")?)?,
            signature: get("signature")?.clone(),
            trace_hash: match get("trace_hash")?.as_str() {
                "none" => None,
                h => Some(u64::from_str_radix(h, 16).map_err(|_| "trace_hash: not hex".to_string())?),
            },
            disagreement: none(get("disagreement")?),
            shrunk,
            message: message.join("\n"),
        })
    }
}

/// What the batch did, and what it found.
#[derive(Clone, Debug)]
pub struct BatchReport {
    pub batch: Batch,
    /// Seeds run. Fewer than asked when the budget ran out.
    pub ran: u64,
    pub passed: u64,
    pub failures: Vec<Record>,
    /// Seeds that passed and cost more than the ceiling, with what they
    /// exceeded.
    pub over_ceiling: Vec<(u64, String)>,
    /// Every passing seed's hash folded into one, in seed order. Two
    /// machines that ran the batch have to agree on it.
    pub sweep_hash: u64,
    /// The dearest passing seed by events per request, and by check
    /// steps.
    pub max_events: Option<(u64, Cost)>,
    pub max_steps: Option<(u64, Cost)>,
    /// Wall-clock time for the whole batch, reported and nothing more.
    pub wall: Duration,
    /// Runs made, counting the reruns and the shrinks.
    pub runs: usize,
    /// The traces of the stable failures, by seed, for the files.
    pub traces: BTreeMap<u64, String>,
}

impl BatchReport {
    /// Nothing failed, nothing disagreed, nothing was over the ceiling.
    pub fn ok(&self) -> bool {
        self.failures.is_empty() && self.over_ceiling.is_empty()
    }

    pub fn unstable(&self) -> impl Iterator<Item = &Record> {
        self.failures.iter().filter(|r| !r.stable())
    }
}

impl fmt::Display for BatchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.batch;
        writeln!(
            f,
            "batch: seeds {}..{} ran={} passed={} failed={} unstable={} over_ceiling={}",
            b.seeds.start,
            b.seeds.end,
            self.ran,
            self.passed,
            self.failures.len(),
            self.unstable().count(),
            self.over_ceiling.len()
        )?;
        writeln!(f, "commit={}", b.commit.as_deref().unwrap_or("none"))?;
        writeln!(
            f,
            "overrides={}",
            if b.overrides.is_empty() {
                "none".to_string()
            } else {
                b.overrides.to_string()
            }
        )?;
        let secs = self.wall.as_secs_f64();
        writeln!(
            f,
            "wall={:.1}s seeds_per_second={:.1} runs={} jobs={}",
            secs,
            if secs > 0.0 { self.ran as f64 / secs } else { 0.0 },
            self.runs,
            b.jobs
        )?;
        if let Some((seed, cost)) = &self.max_events {
            writeln!(
                f,
                "cost: most events per request {} at seed {seed}, ceiling {}",
                cost.events_per_request(),
                b.ceiling.events_per_request
            )?;
        }
        if let Some((seed, cost)) = &self.max_steps {
            writeln!(
                f,
                "cost: most check steps {} at seed {seed}, ceiling {}",
                cost.check_steps, b.ceiling.check_steps
            )?;
        }
        writeln!(f, "sweep_hash={:016x}", self.sweep_hash)?;
        for r in &self.failures {
            match &r.disagreement {
                None => writeln!(
                    f,
                    "failed: seed={} trace_hash={:016x} {}",
                    r.seed,
                    r.trace_hash.unwrap_or(0),
                    r.signature
                )?,
                Some(other) => writeln!(
                    f,
                    "unstable: seed={} first={:?} then={:?}",
                    r.seed, r.signature, other
                )?,
            }
            writeln!(f, "  {}", r.reproduction())?;
            if let Some(shrunk) = &r.shrunk {
                writeln!(f, "  shrunk: {}", reproduction(r.seed, shrunk))?;
            }
        }
        for (seed, what) in &self.over_ceiling {
            writeln!(f, "over ceiling: seed={seed} {what}")?;
        }
        Ok(())
    }
}

/// What the batch reports as it goes.
#[derive(Debug)]
pub enum Progress<'a> {
    Ran(&'a Observed),
    Verified(&'a Record),
    Shrunk(&'a Record),
    OverCeiling(u64, &'a str),
    /// The budget ran out with this many seeds still to run.
    Stopped(u64),
}

/// Run `batch` through `runner`. `log` sees every step.
///
/// The seeds go to the runner `jobs` at a time, in order. Each failure
/// is run twice more with its trace, and the three runs have to agree
/// on the signature, and the two traced runs on their output. Then, if
/// asked, the failure is shrunk. The budget is checked between chunks,
/// so a batch may run a few seeds past it.
pub fn run_batch(batch: &Batch, runner: &mut dyn Runner, mut log: impl FnMut(Progress<'_>)) -> BatchReport {
    let started = Stopwatch::start();
    let mut report = BatchReport {
        batch: batch.clone(),
        ran: 0,
        passed: 0,
        failures: Vec::new(),
        over_ceiling: Vec::new(),
        sweep_hash: 0,
        max_events: None,
        max_steps: None,
        wall: Duration::ZERO,
        runs: 0,
        traces: BTreeMap::new(),
    };
    let mut seeds = batch.seeds.clone();
    let jobs = batch.jobs.max(1);
    loop {
        if let Some(budget) = batch.budget {
            if started.elapsed() >= budget && !seeds.is_empty() {
                log(Progress::Stopped(seeds.end - seeds.start));
                break;
            }
        }
        let chunk: Vec<u64> = seeds.by_ref().take(jobs).collect();
        if chunk.is_empty() {
            break;
        }
        report.runs += chunk.len();
        for observed in runner.run_many(&chunk, &batch.overrides, false) {
            report.ran += 1;
            log(Progress::Ran(&observed));
            match observed.verdict {
                Verdict::Passed { hash, cost } => {
                    report.passed += 1;
                    report.sweep_hash = report.sweep_hash.rotate_left(1) ^ hash;
                    if report
                        .max_events
                        .is_none_or(|(_, c)| cost.events_per_request() > c.events_per_request())
                    {
                        report.max_events = Some((observed.seed, cost));
                    }
                    if report
                        .max_steps
                        .is_none_or(|(_, c)| cost.check_steps > c.check_steps)
                    {
                        report.max_steps = Some((observed.seed, cost));
                    }
                    if let Some(what) = batch.ceiling.exceeded(&cost) {
                        log(Progress::OverCeiling(observed.seed, &what));
                        report.over_ceiling.push((observed.seed, what));
                    }
                }
                Verdict::Failed(failure) => {
                    let record = verify(batch, runner, observed.seed, failure, &mut report);
                    log(Progress::Verified(&record));
                    let record = if batch.shrink && record.stable() {
                        let shrunk = shrink_record(batch, runner, record, &mut report);
                        log(Progress::Shrunk(&shrunk));
                        shrunk
                    } else {
                        record
                    };
                    report.failures.push(record);
                }
            }
        }
    }
    report.wall = started.elapsed();
    report
}

/// Module 1's check on the one run that matters: the failure, twice
/// more, with the trace. The signature has to match the first run, and
/// the two traced runs have to print the same thing.
fn verify(
    batch: &Batch,
    runner: &mut dyn Runner,
    seed: u64,
    failure: Failure,
    report: &mut BatchReport,
) -> Record {
    let signature = failure.signature();
    let mut record = Record {
        seed,
        commit: batch.commit.clone(),
        overrides: batch.overrides.clone(),
        signature: signature.clone(),
        trace_hash: None,
        disagreement: None,
        shrunk: None,
        message: failure.message,
    };
    let mut first: Option<Observed> = None;
    for _ in 0..2 {
        report.runs += 1;
        let Some(again) = runner.run_many(&[seed], &batch.overrides, true).pop() else {
            record.disagreement = Some("no run".into());
            return record;
        };
        let sig = match &again.verdict {
            Verdict::Failed(f) => f.signature(),
            Verdict::Passed { .. } => "pass".to_string(),
        };
        if sig != signature {
            record.disagreement = Some(sig);
            return record;
        }
        match &first {
            None => first = Some(again),
            Some(f) if f.output_hash != again.output_hash => {
                record.disagreement = Some(format!(
                    "the same failure with another trace: {:016x} then {:016x}",
                    f.output_hash, again.output_hash
                ));
                return record;
            }
            Some(_) => {}
        }
    }
    let first = first.expect("two reruns");
    record.trace_hash = Some(first.output_hash);
    if let Some(trace) = first.output {
        report.traces.insert(seed, trace);
    }
    record
}

fn shrink_record(
    batch: &Batch,
    runner: &mut dyn Runner,
    mut record: Record,
    report: &mut BatchReport,
) -> Record {
    let mut oracle = AsOracle(runner);
    match shrink::shrink(record.seed, &batch.overrides, &mut oracle, |_| {}) {
        Ok(shrunk) => {
            report.runs += shrunk.runs;
            record.shrunk = Some(shrunk.result);
        }
        Err(e) => {
            record.disagreement = Some(format!("the shrink found it unstable: {e}"));
        }
    }
    record
}

/// Write the report where CI can pick it up: `summary.txt`, and for
/// every failure `seed-N.txt` with the record and `seed-N.trace` with
/// the trace, ready for `vopr --inspect`.
pub fn write_report(dir: &Path, report: &BatchReport) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    let summary = dir.join("summary.txt");
    fs::write(&summary, report.to_string())?;
    written.push(summary);
    for r in &report.failures {
        let path = dir.join(format!("seed-{}.txt", r.seed));
        fs::write(&path, r.to_text())?;
        written.push(path);
        if let Some(trace) = report.traces.get(&r.seed) {
            let path = dir.join(format!("seed-{}.trace", r.seed));
            fs::write(&path, trace)?;
            written.push(path);
        }
    }
    Ok(written)
}

/// Read `50m`, `90s`, `2h` or a bare number of seconds.
pub fn parse_budget(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (digits, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = digits.parse().ok()?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_names_its_seeds() {
        let a = seeds_for_commit("5b65c2d0f1a2b3c4d5e6f70000000000000000ab", 200).unwrap();
        assert_eq!(a, 0x5b65_c2d0_f1a2..0x5b65_c2d0_f1a2 + 200);
        assert_eq!(seeds_for_commit("5b65c2d0f1a2", 1).unwrap(), a.start..a.start + 1);
        assert_ne!(seeds_for_commit("cc16f1b7e0000", 200).unwrap(), a);
        assert!(seeds_for_commit("5b65c2d", 10).is_err(), "too short");
        assert!(seeds_for_commit("not-a-hash-at-all", 10).is_err());
        assert!(seeds_for_commit("", 10).is_err());
    }

    #[test]
    fn a_pass_and_a_failure_are_read_out_of_a_child() {
        let stdout = "seed=1 nodes=3 ...\nchaos: submitted=150\ncheck: history=180 keys=13 steps=258 cached=210 depth=58\ncost: events=12000 requests=150 per_request=80\ntrace_hash=cc4530a48c93496b\n";
        let o = observe(1, true, stdout, "", Duration::ZERO);
        assert_eq!(
            o.verdict,
            Verdict::Passed {
                hash: 0xcc45_30a4_8c93_496b,
                cost: Cost {
                    events: 12000,
                    requests: 150,
                    check_steps: 258
                }
            }
        );
        assert_eq!(o.output_hash, hash_lines(stdout));
        let stderr = "thread 'main' panicked at x.rs:1:1:\nnode 1 view 7: told to truncate record 2, committed through 2\nnote: run with\n";
        let o = observe(2, false, "seed=2 ...\n", stderr, Duration::ZERO);
        match o.verdict {
            Verdict::Failed(f) => assert_eq!(
                f.signature(),
                "node # view #: told to truncate record #, committed through #"
            ),
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn the_ceiling_names_what_was_exceeded() {
        let c = Ceiling {
            events_per_request: 100,
            check_steps: 1000,
        };
        let fine = Cost {
            events: 5000,
            requests: 50,
            check_steps: 1000,
        };
        assert_eq!(c.exceeded(&fine), None);
        let busy = Cost {
            events: 5050,
            requests: 50,
            ..fine
        };
        assert_eq!(
            c.exceeded(&busy).as_deref(),
            Some("events per request 101 over the ceiling of 100")
        );
        let deep = Cost {
            check_steps: 1001,
            ..fine
        };
        assert!(c.exceeded(&deep).unwrap().starts_with("check steps 1001"));
    }

    #[test]
    fn budgets_parse() {
        assert_eq!(parse_budget("50m"), Some(Duration::from_secs(3000)));
        assert_eq!(parse_budget("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_budget("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_budget("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_budget("7d"), None);
        assert_eq!(parse_budget("m"), None);
    }
}
