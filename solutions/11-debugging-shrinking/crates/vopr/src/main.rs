//! `vopr`: run seeds through the simulator, shrink a failure, and read a
//! trace.
//!
//! ```text
//! vopr --seed N [--trace] [overrides]          one seed, and its trace if asked
//! vopr --seeds A..B [overrides]                every seed from A below B, stop at the first failure
//! vopr --hunt A..B [overrides]                 every seed, each in its own process, report the failures
//! vopr --shrink --seed N [overrides]           a failing seed, reduced to the smallest run that still fails
//! vopr --inspect FILE --at T [--replica N] [--view V] [--since T] [--until T]
//!                                              the cluster at time T, from a trace file
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
//! `panic = "abort"`. So `--shrink` and `--hunt` run every candidate in a
//! child process, this same binary, and read its verdict from its exit
//! status and its stderr.

use determinism::harness::format_hash_line;
use std::process::ExitCode;
use vopr::inspect::{self, Timeline};
use vopr::overrides::{reproduction, Overrides};
use vopr::shrink::{self, Attempt, Oracle as _, Outcome};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  vopr --seed N [--trace] [overrides]\n  vopr --seeds A..B [overrides]\n  vopr --hunt A..B [overrides]\n  vopr --shrink --seed N [overrides]\n  vopr --inspect FILE --at T [--replica N] [--view V] [--since T] [--until T]\n\noverrides: --requests N --nodes 3|5 --clients N --bugs a,b, and --no-<knob> for\n  net disk partitions crashes pauses clocks buggify slow-host core\n  loss dup reorder lost torn misdirect rot read-fault jumps drift offset"
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
}

fn parse_range(s: &str) -> Option<std::ops::Range<u64>> {
    let (a, b) = s.split_once("..")?;
    Some(a.parse().ok()?..b.parse().ok()?)
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
            _ => return None,
        }
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
