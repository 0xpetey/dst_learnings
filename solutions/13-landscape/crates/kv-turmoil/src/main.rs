//! `kv-turmoil`: the replica under turmoil, one seed at a time.
//!
//! ```text
//! kv-turmoil --seed N [--trace] [--requests N] [--bugs a,b]
//! kv-turmoil --seeds A..B [--requests N] [--bugs a,b]
//! ```
//!
//! A seed that passes prints its configuration, a summary and a
//! `trace_hash=` line, as `vopr` does. A seed that fails aborts at the
//! assertion that fired.

use kv_core::Bug;
use kv_turmoil::{run, sweep, RunConfig};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  kv-turmoil --seed N [--trace] [--requests N] [--bugs a,b]\n  kv-turmoil --seeds A..B [--requests N] [--bugs a,b]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = None;
    let mut seeds = None;
    let mut trace = false;
    let mut requests = 100;
    let mut bugs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args.get(i).and_then(|s| s.parse().ok());
                if seed.is_none() {
                    return usage();
                }
            }
            "--seeds" => {
                i += 1;
                let Some((a, b)) = args.get(i).and_then(|s| s.split_once("..")) else {
                    return usage();
                };
                match (a.parse::<u64>(), b.parse::<u64>()) {
                    (Ok(a), Ok(b)) if a <= b => seeds = Some(a..b),
                    _ => return usage(),
                }
            }
            "--requests" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse().ok()) {
                    Some(n) => requests = n,
                    None => return usage(),
                }
            }
            "--bugs" => {
                i += 1;
                let Some(list) = args.get(i) else {
                    return usage();
                };
                for name in list.split(',') {
                    match Bug::from_name(name) {
                        Some(bug) => bugs.push(bug),
                        None => {
                            eprintln!("unknown bug: {name}");
                            return usage();
                        }
                    }
                }
            }
            "--trace" => trace = true,
            _ => return usage(),
        }
        i += 1;
    }
    match (seed, seeds) {
        (Some(seed), None) => {
            let mut config = RunConfig::from_seed(seed, requests);
            config.bugs = bugs;
            config.echo_trace = trace;
            println!("{config}");
            let report = run(&config);
            println!("{report}");
            println!("trace_hash={:016x}", report.hash);
            ExitCode::SUCCESS
        }
        (None, Some(seeds)) => {
            let count = seeds.end - seeds.start;
            let hash = sweep(seeds, requests, &bugs);
            println!("{count} seeds passed");
            println!("trace_hash={hash:016x}");
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
