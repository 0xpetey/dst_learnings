//! `vopr`: run seeds through the simulator.
//!
//! ```text
//! vopr --seed N [--requests N] [--trace]      one seed, and its trace if asked
//! vopr --seeds A..B [--requests N]            every seed from A below B, stop at the first failure
//! ```
//!
//! A seed that passes prints its configuration, a summary and a
//! `trace_hash=` line. A seed that fails prints its configuration and then
//! aborts at the assertion that fired, which is the whole report: the seed
//! to rerun, the configuration it meant, and the check that failed. With
//! `--trace` every line of the trace is printed as it happens, so a
//! failing seed shows everything up to the assertion.

use determinism::harness::format_hash_line;
use std::process::ExitCode;
use vopr::RunConfig;

fn usage() -> ExitCode {
    eprintln!("usage:\n  vopr --seed N [--requests N] [--trace]\n  vopr --seeds A..B [--requests N]");
    ExitCode::from(2)
}

struct Args {
    seeds: std::ops::Range<u64>,
    requests: u64,
    trace: bool,
}

fn parse(args: &[String]) -> Option<Args> {
    let mut seeds = None;
    let mut requests = 150;
    let mut trace = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                let seed: u64 = args.get(i + 1)?.parse().ok()?;
                seeds = Some(seed..seed + 1);
                i += 2;
            }
            "--seeds" => {
                let (a, b) = args.get(i + 1)?.split_once("..")?;
                seeds = Some(a.parse().ok()?..b.parse().ok()?);
                i += 2;
            }
            "--requests" => {
                requests = args.get(i + 1)?.parse().ok()?;
                i += 2;
            }
            "--trace" => {
                trace = true;
                i += 1;
            }
            _ => return None,
        }
    }
    Some(Args {
        seeds: seeds?,
        requests,
        trace,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(args) = parse(&args) else {
        return usage();
    };
    let one = args.seeds.end == args.seeds.start + 1;
    // The hash of the sweep: every seed's hash folded into one, so that
    // `vopr --seeds 0..100` is itself checkable for determinism.
    let mut sweep: u64 = 0;
    for seed in args.seeds {
        let mut config = RunConfig::from_seed(seed, args.requests);
        // The trace is printed as it happens, so that a failing seed is
        // readable up to the assertion. The configuration line comes
        // first, then the world's own two configuration lines.
        config.world.echo_trace = args.trace;
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
