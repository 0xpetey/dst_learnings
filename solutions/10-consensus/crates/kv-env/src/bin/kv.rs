//! `kv`: the real KV store on the command line.
//!
//! ```text
//! kv serve <addr> <data-dir> [--verbose]
//! kv get <addr> <key>
//! kv put <addr> <key> <value>
//! kv delete <addr> <key>
//! ```

use kv_core::client::Config;
use kv_core::{Request, Response};
use kv_env::real;
use std::path::Path;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage:\n  kv serve <addr> <data-dir> [--verbose]\n  kv get <addr> <key>\n  kv put <addr> <key> <value>\n  kv delete <addr> <key>");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match (arg(0), arg(1), arg(2), arg(3)) {
        (Some("serve"), Some(addr), Some(dir), verbose) => {
            let verbose = verbose == Some("--verbose");
            match real::serve(addr, Path::new(dir), verbose, |bound| {
                println!("listening on {bound}")
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("server failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        (Some(op @ ("get" | "put" | "delete")), Some(addr), Some(key), value) => {
            let request = match (op, value) {
                ("get", None) => Request::Get { key: key.into() },
                ("put", Some(v)) => Request::Put {
                    key: key.into(),
                    value: v.into(),
                },
                ("delete", None) => Request::Delete { key: key.into() },
                _ => return usage(),
            };
            // A one-shot process is its own session; the id only needs to be
            // unique among concurrent clients of this server.
            let client_id = client_id_for_process();
            match real::call(addr, client_id, request, Config::default()) {
                Ok(Response::Value(Some(v))) => {
                    println!("{}", String::from_utf8_lossy(&v));
                    ExitCode::SUCCESS
                }
                Ok(Response::Value(None)) => {
                    println!("(nil)");
                    ExitCode::SUCCESS
                }
                Ok(Response::PutOk) => {
                    println!("OK");
                    ExitCode::SUCCESS
                }
                Ok(Response::DeleteOk { existed }) => {
                    println!("{}", if existed { "1" } else { "0" });
                    ExitCode::SUCCESS
                }
                Ok(Response::Rejected(why)) => {
                    eprintln!("rejected: {why:?}");
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}

/// Process id mixed with the wall clock. This is the real environment, so a
/// nondeterministic id is exactly right here and exactly wrong anywhere else.
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "real one-shot client; a random session id is wanted here"
)]
fn client_id_for_process() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (u64::from(std::process::id()) << 32) ^ nanos
}
