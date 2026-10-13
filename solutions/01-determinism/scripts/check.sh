#!/usr/bin/env bash
# The whole acceptance suite for Module 1, in the order CI runs it.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== fmt"
cargo fmt --all -- --check

echo "== clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "== tests"
cargo test --workspace

echo "== the same seed prints the same line in two processes"
cargo build -q --bin trace_hash
first=$(target/debug/trace_hash --seed 7)
second=$(target/debug/trace_hash --seed 7)
echo "$first"
case "$first" in
  trace_hash=????????????????) ;;
  *) echo "expected 'trace_hash=' and sixteen hex digits, got: $first"; exit 1 ;;
esac
if [ "$first" != "$second" ]; then
  echo "seed 7 printed two different lines: $first / $second"
  exit 1
fi

echo "== an address hazard changes the line between processes"
# ASLR moves the heap on every run, so three runs that all agree mean the
# hazard is not reaching the trace.
a=$(target/debug/trace_hash --seed 7 --hazard address)
b=$(target/debug/trace_hash --seed 7 --hazard address)
c=$(target/debug/trace_hash --seed 7 --hazard address)
if [ "$a" = "$b" ] && [ "$b" = "$c" ]; then
  echo "three runs with --hazard address printed the same line: $a"
  exit 1
fi

echo "== the hazard tests pass twenty times in a row"
# A thread test that passes 19 of 20 times has a barrier or workload that is
# too small. Each failure prints the test's output.
for i in $(seq 1 20); do
  cargo test -q -p determinism --test hazards > /dev/null || {
    echo "hazards failed on run $i of 20"
    cargo test -p determinism --test hazards
    exit 1
  }
done

echo "== all checks passed"
