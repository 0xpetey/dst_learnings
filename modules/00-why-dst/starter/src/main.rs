//! Module 0 starter: a counter with a lost update under retry-with-timeout.
//!
//! Layout:
//!   - one `store` thread owns the counters and answers Get/Put requests
//!   - several `client` threads increment counters by Get, then Put(value + 1)
//!   - every request waits for its reply with a timeout and is resent on timeout
//!
//! Key 0 is shared by all clients. Key `id + 1` is private to client `id`.
//! `main` checks the final counters against what they must be and exits
//! non-zero if they differ.
//!
//! Fill in the `TODO`s. The skeleton compiles as is and reports FAIL because
//! the stubs never increment anything. Remove the `allow(dead_code)` below once
//! the stubs are filled in.

#![allow(dead_code)]

use std::env;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Run parameters, overridable from the command line:
/// `lost_update [clients] [increments_per_client] [shared_every] [timeout_us]`.
#[derive(Clone, Copy, Debug)]
struct Config {
    /// Number of client threads. Each also owns one private key.
    clients: usize,
    /// Increments each client performs.
    increments_per_client: u64,
    /// Every Nth increment goes to shared key 0; the rest go to the private key.
    shared_every: u64,
    /// How long a client waits for a reply before resending the request.
    timeout: Duration,
}

const DEFAULT: Config = Config {
    clients: 2,
    increments_per_client: 2_000,
    shared_every: 500,
    timeout: Duration::from_micros(50),
};

#[derive(Clone, Copy, Debug)]
enum Op {
    Get { key: usize },
    Put { key: usize, value: u64 },
}

struct Request {
    op: Op,
    /// Where the store sends the reply. Each client owns one reply channel
    /// and reuses it for every request it sends.
    reply: Sender<u64>,
}

struct ClientStats {
    /// How many times a timeout fired and the request was resent.
    retries: u64,
}

/// The store thread. Processes requests in arrival order until every sender
/// is gone, then returns the final counters.
fn store(rx: Receiver<Request>, keys: usize) -> Vec<u64> {
    let values = vec![0u64; keys];
    // TODO: for each request, apply it to `values` and send the result back
    // on `req.reply`. Get replies with the current value; Put stores the
    // value and replies with it. Ignore a failed reply send (the client may
    // be gone).
    for _req in rx {}
    values
}

/// Send one request and wait for its reply. If the timeout fires, resend the
/// same request and count a retry.
fn call(
    _tx: &Sender<Request>,
    _reply_tx: &Sender<u64>,
    _reply_rx: &Receiver<u64>,
    _op: Op,
    _timeout: Duration,
    _stats: &mut ClientStats,
) -> u64 {
    // TODO: loop { send the request; recv_timeout; on Ok return the value;
    // on Timeout bump stats.retries and loop; on Disconnected panic }.
    0
}

/// One client: `increments_per_client` read-modify-write increments, most on
/// its private key and every `shared_every`th one on the shared key 0.
fn client(id: usize, cfg: Config, tx: Sender<Request>) -> ClientStats {
    let (_reply_tx, _reply_rx) = mpsc::channel::<u64>();
    let stats = ClientStats { retries: 0 };
    let _private_key = id + 1;
    let _ = (cfg, tx);
    // TODO: for i in 0..cfg.increments_per_client:
    //   key = 0 if i % cfg.shared_every == 0 else private_key
    //   current = call(Get { key })
    //   call(Put { key, value: current + 1 })
    stats
}

fn parse_config() -> Config {
    let mut cfg = DEFAULT;
    let args: Vec<String> = env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let num = |i: usize| -> Option<u64> { args.get(i).and_then(|a| a.parse().ok()) };
    if let Some(v) = num(0) {
        cfg.clients = v as usize;
    }
    if let Some(v) = num(1) {
        cfg.increments_per_client = v;
    }
    if let Some(v) = num(2) {
        cfg.shared_every = v.max(1);
    }
    if let Some(v) = num(3) {
        cfg.timeout = Duration::from_micros(v);
    }
    cfg
}

fn main() -> ExitCode {
    let verbose = env::args().any(|a| a == "-v" || a == "--verbose");
    let cfg = parse_config();
    let started = Instant::now();

    // Key 0 is shared; keys 1..=clients are private.
    let keys = cfg.clients + 1;
    let (tx, rx) = mpsc::channel::<Request>();
    let store_handle = thread::spawn(move || store(rx, keys));

    let clients: Vec<_> = (0..cfg.clients)
        .map(|id| {
            let tx = tx.clone();
            thread::spawn(move || client(id, cfg, tx))
        })
        .collect();
    // Drop the main thread's sender so the store exits once clients finish.
    drop(tx);

    let mut total_retries = 0;
    for handle in clients {
        let stats = handle.join().expect("client panicked");
        total_retries += stats.retries;
    }
    let values = store_handle.join().expect("store panicked");

    // What the counters must be if every increment took effect exactly once.
    let shared_per_client = cfg.increments_per_client.div_ceil(cfg.shared_every);
    let expected_shared = shared_per_client * cfg.clients as u64;
    let expected_private = cfg.increments_per_client - shared_per_client;

    let mut failures = Vec::new();
    if values[0] != expected_shared {
        failures.push(format!(
            "shared key 0: got {} expected {} ({} lost)",
            values[0],
            expected_shared,
            expected_shared as i64 - values[0] as i64
        ));
    }
    for id in 0..cfg.clients {
        let key = id + 1;
        if values[key] != expected_private {
            failures.push(format!(
                "private key {}: got {} expected {} (diff {})",
                key,
                values[key],
                expected_private,
                values[key] as i64 - expected_private as i64
            ));
        }
    }

    let elapsed = started.elapsed();
    if verbose || !failures.is_empty() {
        eprintln!("config={cfg:?}");
        eprintln!("values={values:?} retries={total_retries} elapsed={elapsed:?}");
    }
    if failures.is_empty() {
        println!("OK   retries={total_retries}");
        ExitCode::SUCCESS
    } else {
        for f in &failures {
            println!("FAIL {f}");
        }
        ExitCode::FAILURE
    }
}
