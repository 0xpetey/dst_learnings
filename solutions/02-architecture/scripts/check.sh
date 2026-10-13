#!/usr/bin/env bash
# The whole acceptance suite for Module 2, in the order CI runs it.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== fmt"
cargo fmt --all -- --check

echo "== clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "== tests (including the real TCP test)"
cargo test --workspace

echo "== kv-core reaches no OS module"
# Comments do not count, code does. grep exits 1 when nothing matches, which
# is the pass here.
if grep -rn "std::time\|std::thread\|std::fs\|std::net\|std::env" crates/kv-core/src | grep -v "//"; then
  echo "kv-core names an OS module"
  exit 1
fi

echo "== a value survives a server restart"
cargo build -q --bin kv
data=$(mktemp -d)
log=$(mktemp)
server=""
cleanup() {
  if [ -n "$server" ]; then kill "$server" 2> /dev/null || true; fi
  rm -rf "$data" "$log"
}
trap cleanup EXIT

# Port 0 asks the OS for a free port, and the server prints the one it got.
start_server() {
  : > "$log"
  target/debug/kv serve 127.0.0.1:0 "$data" > "$log" 2>&1 &
  server=$!
  for _ in $(seq 1 100); do
    if grep -q "^listening on " "$log"; then break; fi
    sleep 0.05
  done
  addr=$(sed -n 's/^listening on //p' "$log")
  if [ -z "$addr" ]; then
    echo "the server did not print its address"
    cat "$log"
    exit 1
  fi
}
stop_server() {
  kill "$server"
  wait "$server" 2> /dev/null || true
  server=""
}

start_server
target/debug/kv put "$addr" greeting hello > /dev/null
got=$(target/debug/kv get "$addr" greeting)
if [ "$got" != "hello" ]; then
  echo "get before restart printed '$got', expected 'hello'"
  exit 1
fi
stop_server

start_server
got=$(target/debug/kv get "$addr" greeting)
if [ "$got" != "hello" ]; then
  echo "get after restart printed '$got', expected 'hello'"
  exit 1
fi
stop_server

echo "== all checks passed"
