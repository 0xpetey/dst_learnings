//! `vopr`: run seeds through the simulator, shrink a failure, read a
//! trace, and run the batch that CI runs.
//!
//! ```text
//! vopr --seed N [--trace] [overrides]          one seed, and its trace if asked
//! vopr --seeds A..B [overrides]                every seed from A below B, stop at the first failure
//! vopr --hunt A..B [overrides]                 every seed, each in its own process, report the failures
//! vopr --shrink --seed N [overrides]           a failing seed, reduced to the smallest run that still fails
//! vopr --inspect FILE --at T [--replica N] [--view V] [--since T] [--until T]
//!                                              the cluster at time T, from a trace file
//! vopr --batch COUNT [--from N] [--commit SHA] [--jobs J] [--for 50m] [--shrink]
//!                    [--report DIR] [--max-events N] [--max-steps N] [overrides]
//!                                              COUNT seeds from N, or from where the commit's hash
//!                                              says, J at a time, every failure verified twice,
//!                                              a file per failure
//!
//! overrides: --requests N --nodes 3|5 --clients N --bugs a,b
//!            --no-net --no-disk --no-partitions --no-crashes --no-pauses --no-clocks
//!            --no-buggify --no-slow-host --no-core --no-loss --no-dup --no-reorder
//!            --no-lost --no-torn --no-misdirect --no-rot --no-read-fault
//!            --no-jumps --no-drift --no-offset
//! ```
//!
//! A seed that passes prints its configuration, a summary and a
//! `trace_hash=` line. A seed that fails prints its configuration and then
//! aborts at the assertion that fired, which is the whole report: the seed
//! to rerun, the configuration it meant, and the check that failed. With
//! `--trace` every line of the trace is printed as it happens, so a
//! failing seed shows everything up to the assertion.
//!
//! The process aborts at a failure, because the workspace says
//! `panic = "abort"`. So `--shrink`, `--hunt` and `--batch` run every
//! candidate in a child process, this same binary, and read its verdict
//! from its exit status and its stderr.

use determinism::harness::format_hash_line;
use std::path::PathBuf;
use std::process::ExitCode;
use vopr::batch::{self, Batch, Ceiling, Progress, Verdict};
use vopr::inspect::{self, Timeline};
use vopr::overrides::{reproduction, Overrides};
use vopr::shrink::{self, Attempt, Oracle as _, Outcome};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  vopr --seed N [--trace] [overrides]\n  vopr --seeds A..B [overrides]\n  vopr --hunt A..B [overrides]\n  vopr --shrink --seed N [overrides]\n  vopr --inspect FILE --at T [--replica N] [--view V] [--since T] [--until T]\n  vopr --batch COUNT [--from N] [--commit SHA] [--jobs J] [--for 50m] [--shrink]\n       [--report DIR] [--max-events N] [--max-steps N] [overrides]\n\noverrides: --requests N --nodes 3|5 --clients N --bugs a,b, and --no-<knob> for\n  net disk partitions crashes pauses clocks buggify slow-host core\n  loss dup reorder lost torn misdirect rot read-fault jumps drift offset"
    );
    ExitCode::from(2)
}

enum Mode {
    Run {
        seeds: std::ops::Range<u64>,
        trace: bool,
    },
    Hunt {
        seeds: std::ops::Range<u64>,
    },
    Shrink {
        seed: u64,
    },
    Inspect(inspect::Query),
    Batch {
        batch: Batch,
        report: Option<PathBuf>,
    },
}

fn parse_range(s: &str) -> Option<std::ops::Range<u64>> {
    let (a, b) = s.split_once("..")?;
    Some(a.parse().ok()?..b.parse().ok()?)
}

/// What `--batch` takes, gathered before the batch is built.
#[derive(Default)]
struct BatchArgs {
    count: Option<u64>,
    from: Option<u64>,
    commit: Option<String>,
    jobs: Option<usize>,
    budget: Option<std::time::Duration>,
    report: Option<PathBuf>,
    max_events: Option<u64>,
    max_steps: Option<u64>,
}

impl BatchArgs {
    fn build(self, overrides: Overrides, shrink: bool) -> Option<(Batch, Option<PathBuf>)> {
        let count = self.count?;
        // The seeds start at `--from`, or where the commit's hash says.
        // The commit is recorded either way.
        let seeds = match (self.from, &self.commit) {
            (Some(from), _) => from..from.checked_add(count)?,
            (None, Some(commit)) => match batch::seeds_for_commit(commit, count) {
                Ok(seeds) => seeds,
                Err(e) => {
                    eprintln!("{e}");
                    return None;
                }
            },
            (None, None) => {
                eprintln!("--batch takes --from N, or --commit SHA to start where the hash says");
                return None;
            }
        };
        let mut ceiling = Ceiling::default();
        if let Some(n) = self.max_events {
            ceiling.events_per_request = n;
        }
        if let Some(n) = self.max_steps {
            ceiling.check_steps = n;
        }
        let batch = Batch {
            seeds,
            overrides,
            commit: self.commit,
            budget: self.budget,
            ceiling,
            shrink,
            jobs: self.jobs.unwrap_or(1),
        };
        Some((batch, self.report))
    }
}

fn parse(args: &[String]) -> Option<(Mode, Overrides)> {
    let (overrides, rest) = match Overrides::parse(args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return None;
        }
    };
    let mut seeds = None;
    let mut hunt = None;
    let mut trace = false;
    let mut shrink = false;
    let mut inspect: Option<inspect::Query> = None;
    let mut batch = BatchArgs::default();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--seed" => {
                let seed: u64 = rest.get(i + 1)?.parse().ok()?;
                seeds = Some(seed..seed + 1);
                i += 2;
            }
            "--seeds" => {
                seeds = Some(parse_range(rest.get(i + 1)?)?);
                i += 2;
            }
            "--hunt" => {
                hunt = Some(parse_range(rest.get(i + 1)?)?);
                i += 2;
            }
            "--trace" => {
                trace = true;
                i += 1;
            }
            "--shrink" => {
                shrink = true;
                i += 1;
            }
            "--inspect" => {
                inspect = Some(inspect::Query::new(rest.get(i + 1)?));
                i += 2;
            }
            "--at" | "--since" | "--until" | "--replica" | "--view" => {
                let q = inspect.as_mut()?;
                let v = rest.get(i + 1)?;
                match rest[i].as_str() {
                    "--at" => q.at = Some(inspect::parse_time(v)?),
                    "--since" => q.since = Some(inspect::parse_time(v)?),
                    "--until" => q.until = Some(inspect::parse_time(v)?),
                    "--replica" => q.replica = Some(v.trim_start_matches('n').parse().ok()?),
                    _ => q.view = Some(v.parse().ok()?),
                }
                i += 2;
            }
            "--batch" | "--from" | "--commit" | "--jobs" | "--for" | "--report" | "--max-events"
            | "--max-steps" => {
                let v = rest.get(i + 1)?;
                match rest[i].as_str() {
                    "--batch" => batch.count = Some(v.parse().ok()?),
                    "--from" => batch.from = Some(v.parse().ok()?),
                    "--commit" => batch.commit = Some(v.clone()),
                    "--jobs" => batch.jobs = Some(v.parse().ok()?),
                    "--for" => batch.budget = Some(batch::parse_budget(v)?),
                    "--report" => batch.report = Some(PathBuf::from(v)),
                    "--max-events" => batch.max_events = Some(v.parse().ok()?),
                    _ => batch.max_steps = Some(v.parse().ok()?),
                }
                i += 2;
            }
            _ => return None,
        }
    }
    if batch.count.is_some() {
        if seeds.is_some() || hunt.is_some() || inspect.is_some() || trace {
            return None;
        }
        let (batch, report) = batch.build(overrides.clone(), shrink)?;
        return Some((Mode::Batch { batch, report }, overrides));
    }
    let mode = match (seeds, hunt, shrink, inspect) {
        (Some(seeds), None, false, None) => Mode::Run { seeds, trace },
        (None, Some(seeds), false, None) => Mode::Hunt { seeds },
        (Some(seeds), None, true, None) if seeds.end == seeds.start + 1 => Mode::Shrink { seed: seeds.start },
        (None, None, false, Some(q)) => Mode::Inspect(q),
        _ => return None,
    };
    Some((mode, overrides))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((mode, overrides)) = parse(&args) else {
        return usage();
    };
    match mode {
        Mode::Run { seeds, trace } => run(seeds, trace, &overrides),
        Mode::Hunt { seeds } => hunt(seeds, &overrides),
        Mode::Shrink { seed } => shrink(seed, &overrides),
        Mode::Inspect(query) => inspect(&query),
        Mode::Batch { batch, report } => run_batch(&batch, report.as_deref()),
    }
}

fn run(seeds: std::ops::Range<u64>, trace: bool, overrides: &Overrides) -> ExitCode {
    let one = seeds.end == seeds.start + 1;
    // The hash of the sweep: every seed's hash folded into one, so that
    // `vopr --seeds 0..100` is itself checkable for determinism.
    let mut sweep: u64 = 0;
    for seed in seeds {
        let mut config = overrides.configure(seed);
        // The trace is printed as it happens, so that a failing seed is
        // readable up to the assertion. The configuration line comes
        // first, then the world's own two configuration lines.
        config.world.echo_trace = trace;
        println!("{config}");
        let report = vopr::run(&config);
        if one {
            println!("{report}");
        } else {
            println!(
                "seed={seed} ok chaos={}/{} converged_in={:?} view_changes={}",
                report.chaos.completed,
                report.chaos.submitted,
                report.converged_in,
                report.replicas.iter().map(|r| r.view_changes).sum::<u64>()
            );
        }
        sweep = sweep.rotate_left(1) ^ report.hash;
    }
    println!("{}", format_hash_line(sweep));
    ExitCode::SUCCESS
}

/// Every seed in its own process. Nothing stops the hunt, and the
/// failures are listed at the end with their reproductions.
fn hunt(seeds: std::ops::Range<u64>, overrides: &Overrides) -> ExitCode {
    let mut oracle = shrink::ChildProcess::of_this_binary();
    let mut failures = Vec::new();
    let total = seeds.end.saturating_sub(seeds.start);
    for seed in seeds {
        match oracle.run(seed, overrides) {
            None => println!("seed={seed} ok"),
            Some(failure) => {
                println!("seed={seed} FAILED {}", failure.signature());
                failures.push((seed, failure));
            }
        }
    }
    println!("hunt: {} of {total} seeds failed", failures.len());
    for (seed, failure) in &failures {
        println!("  {}", reproduction(*seed, overrides));
        println!("    {}", failure.first_line());
    }
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn shrink(seed: u64, start: &Overrides) -> ExitCode {
    let mut oracle = shrink::ChildProcess::of_this_binary();
    let log = |attempt: &Attempt| {
        let verdict = match &attempt.outcome {
            Outcome::Kept => "kept".to_string(),
            Outcome::Passed => "passed".to_string(),
            Outcome::Other(sig) => format!("other failure: {sig}"),
        };
        println!("  try {:<40} {verdict}", attempt.change);
    };
    match shrink::shrink(seed, start, &mut oracle, log) {
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
        Ok(result) => {
            println!("{result}");
            ExitCode::SUCCESS
        }
    }
}

fn inspect(query: &inspect::Query) -> ExitCode {
    let text = match std::fs::read_to_string(&query.file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {e}", query.file);
            return ExitCode::FAILURE;
        }
    };
    let timeline = Timeline::parse(text.lines());
    print!("{}", timeline.render(query));
    ExitCode::SUCCESS
}

/// The batch CI runs: `jobs` children at a time, every failure verified
/// twice with its trace, a report directory at the end. The exit status
/// is the verdict, and it is non-zero for a failure, for a run that
/// disagreed with itself, and for a run over the cost ceiling.
fn run_batch(batch: &Batch, report_dir: Option<&std::path::Path>) -> ExitCode {
    let mut runner = batch::Children::of_this_binary(batch.jobs);
    let log = |p: Progress<'_>| match p {
        Progress::Ran(o) => match &o.verdict {
            Verdict::Passed { cost, .. } => println!(
                "seed={} ok events/request={} steps={} in {:.2}s",
                o.seed,
                cost.events_per_request(),
                cost.check_steps,
                o.wall.as_secs_f64()
            ),
            Verdict::Failed(f) => println!("seed={} FAILED {}", o.seed, f.signature()),
        },
        Progress::Verified(r) => match &r.disagreement {
            None => println!(
                "seed={} verified twice, trace_hash={:016x}",
                r.seed,
                r.trace_hash.unwrap_or(0)
            ),
            Some(other) => println!("seed={} UNSTABLE: {other}", r.seed),
        },
        Progress::Shrunk(r) => match &r.shrunk {
            Some(s) => println!("seed={} shrunk to: {}", r.seed, reproduction(r.seed, s)),
            None => println!("seed={} not shrunk", r.seed),
        },
        Progress::OverCeiling(seed, what) => println!("seed={seed} OVER CEILING: {what}"),
        Progress::Stopped(left) => println!("budget spent with {left} seeds left"),
    };
    let report = batch::run_batch(batch, &mut runner, log);
    print!("{report}");
    if let Some(dir) = report_dir {
        match batch::write_report(dir, &report) {
            Ok(files) => println!("report: {} files in {}", files.len(), dir.display()),
            Err(e) => {
                eprintln!("could not write the report to {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
