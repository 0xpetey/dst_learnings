//! A tiny program that runs under a seed and prints its trace hash, in the
//! format the cross-process determinism check expects.
//!
//! Usage: cargo run --bin trace_hash -- [--hazard none|address|env] --seed N
//!
//! With `--hazard address` the trace includes a heap address, which ASLR
//! changes every process. With `--hazard env` it includes an environment
//! variable. Both pass an in-process check and fail a cross-process one.

use determinism::harness::{format_hash_line, seed_from_args};
use determinism::{trace, Prng, Trace};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed = seed_from_args(args.iter().cloned(), 0);
    let hazard = args
        .iter()
        .position(|a| a == "--hazard")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("none");

    let mut prng = Prng::from_seed(seed);
    let mut trace = Trace::new();

    let nodes: Vec<Box<u64>> = (0..3).map(|i| Box::new(prng.next_u64() ^ i)).collect();
    for (i, node) in nodes.iter().enumerate() {
        match hazard {
            "address" => trace!(trace, "node {i} at {:p} value {}", &**node, node),
            "env" => trace!(
                trace,
                "node {i} on host {} value {}",
                std::env::var("DST_HOSTNAME").unwrap_or_default(),
                node
            ),
            _ => trace!(trace, "node {i} value {}", node),
        }
    }
    println!("{}", format_hash_line(trace.hash()));
}
