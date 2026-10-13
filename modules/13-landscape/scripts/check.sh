#!/usr/bin/env bash
# The whole acceptance suite, in the order CI runs it.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== fmt"
cargo fmt --all -- --check

echo "== clippy (bans from clippy.toml are deny)"
# --locked everywhere: the lockfile in the commit is the one that builds,
# or the build fails. A build that resolves a dependency anew is not the
# same binary, and a seed names a run of one binary.
cargo clippy --locked --workspace --all-targets -- -D warnings

echo "== core crates build with no std at all"
# A bare-metal target has no std to fall back on; if this compiles, nothing
# in these crates can reach the OS. Install once with:
#   rustup target add thumbv7em-none-eabihf
cargo check --locked -p kv-core --target thumbv7em-none-eabihf
# With fault points and planted bugs compiled in, the core is still free of std.
cargo check --locked -p kv-core --features buggify,bugs --target thumbv7em-none-eabihf
cargo check --locked -p sim --target thumbv7em-none-eabihf
# The oracle has no opinion of its own about time or hashing either.
cargo check --locked -p checker --target thumbv7em-none-eabihf
cargo check --locked -p determinism --no-default-features --target thumbv7em-none-eabihf

echo "== tests"
cargo test --locked --workspace

echo "== hazards cannot be reintroduced into kv-core"
scripts/hazard-check.sh

echo "== the same run in debug and in release"
# Both profiles keep overflow checks and debug assertions on, so the one
# seed means one run in either. The tests run in debug and the batches in
# release, and this is what lets a seed cross between them.
debug=$(cargo run --locked -q -p vopr -- --seed 1 | tail -1)
release=$(cargo run --release --locked -q -p vopr -- --seed 1 | tail -1)
echo "debug   $debug"
echo "release $release"
test "$debug" = "$release"

echo "== the simulator, two hundred seeds"
# A seed that fails prints its configuration and the assertion that fired.
# Rerun it alone with `cargo run --release -p vopr -- --seed N --trace`,
# and shrink it with `cargo run --release -p vopr -- --shrink --seed N`.
sweep=$(cargo run --release --locked -q -p vopr -- --seeds 0..200 | tail -1)
echo "$sweep"

echo "== the shrinker, on the planted Module 10 bug"
# Seed 9916 restarts a replica without its view promise. The shrinker
# must still reach a shorter run with the same guard firing.
cargo run --release --locked -q -p vopr -- --shrink --seed 9916 --nodes 3 --no-core --bugs forgotten_promise | tail -2

echo "== the batch, as CI runs it"
# The same two hundred seeds, four at a time in child processes, under
# the cost ceiling. The sweep hash has to equal the one-process hash.
batch=$(cargo run --release --locked -q -p vopr -- --batch 200 --from 0 --jobs 4 | tail -4)
echo "$batch"
test "${sweep#trace_hash=}" = "$(echo "$batch" | tail -1 | sed 's/^sweep_hash=//')"

echo "== the replica under turmoil, twenty seeds"
# Module 13: the same replica on somebody else's simulator. A seed that
# fails aborts at its assertion, as vopr does.
cargo run --release --locked -q -p kv-turmoil -- --seeds 0..20 --requests 50 | tail -1

echo "== all checks passed"
