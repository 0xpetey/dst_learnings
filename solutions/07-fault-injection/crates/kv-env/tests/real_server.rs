//! The same node behind real TCP and a real file. Spawns the `kv` binary,
//! talks to it with the `kv` client, restarts it, and checks durability.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Server {
    child: Child,
    addr: String,
}

impl Server {
    fn start(data_dir: &Path) -> Server {
        // Pick a free port, then hand it to the server. Racy in theory, fine
        // for a test on one machine.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr = format!("127.0.0.1:{port}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_kv"))
            .args(["serve", &addr, data_dir.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn kv serve");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(line.starts_with("listening on"), "server did not start: {line}");
        Server { child, addr }
    }

    fn kv(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_kv"))
            .arg(args[0])
            .arg(&self.addr)
            .args(&args[1..])
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kv-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn end_to_end_over_tcp_with_restart() {
    let dir = temp_dir("e2e");
    {
        let s = Server::start(&dir);
        assert_eq!(s.kv(&["get", "a"]), (true, "(nil)".into()));
        assert_eq!(s.kv(&["put", "a", "hello"]), (true, "OK".into()));
        assert_eq!(s.kv(&["put", "b", "world"]), (true, "OK".into()));
        assert_eq!(s.kv(&["get", "a"]), (true, "hello".into()));
        assert_eq!(s.kv(&["delete", "a"]), (true, "1".into()));
        assert_eq!(s.kv(&["delete", "a"]), (true, "0".into()));
        assert_eq!(s.kv(&["get", "a"]), (true, "(nil)".into()));
    }
    // Server killed. Restart on the same data directory.
    {
        let s = Server::start(&dir);
        assert_eq!(s.kv(&["get", "a"]), (true, "(nil)".into()));
        assert_eq!(s.kv(&["get", "b"]), (true, "world".into()));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rejected_requests_fail_cleanly() {
    let dir = temp_dir("reject");
    let s = Server::start(&dir);
    let long_key = "k".repeat(2000);
    let (ok, _) = s.kv(&["get", &long_key]);
    assert!(!ok);
    // The server is still alive afterwards.
    assert_eq!(s.kv(&["get", "a"]), (true, "(nil)".into()));
    let _ = std::fs::remove_dir_all(&dir);
}
