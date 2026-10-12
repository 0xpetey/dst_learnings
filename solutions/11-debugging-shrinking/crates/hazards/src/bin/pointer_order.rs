//! Prints the trace hash of the pointer-order hazard (or its fix), so the
//! cross-process determinism check can compare two processes.
//!
//! Usage: pointer_order [--fixed] --seed N

use determinism::harness::{format_hash_line, seed_from_args};
use determinism::{Prng, Trace};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fixed = args.iter().any(|a| a == "--fixed");
    let seed = seed_from_args(args, 0);
    let mut prng = Prng::from_seed(seed);
    let mut trace = Trace::new();
    if fixed {
        hazards::fixed_pointer_order(&mut prng, &mut trace);
    } else {
        hazards::hazard_pointer_order(&mut prng, &mut trace);
    }
    println!("{}", format_hash_line(trace.hash()));
}
