#!/usr/bin/env bash
# The whole acceptance suite, in the order CI runs it.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== fmt"
cargo fmt --all -- --check

echo "== clippy (bans from clippy.toml are deny)"
cargo clippy --workspace --all-targets -- -D warnings

echo "== core crates build with no std at all"
# A bare-metal target has no std to fall back on; if this compiles, nothing
# in these crates can reach the OS. Install once with:
#   rustup target add thumbv7em-none-eabihf
cargo check -p kv-core --target thumbv7em-none-eabihf
# With fault points and planted bugs compiled in, the core is still free of std.
cargo check -p kv-core --features buggify,bugs --target thumbv7em-none-eabihf
cargo check -p sim --target thumbv7em-none-eabihf
# The oracle has no opinion of its own about time or hashing either.
cargo check -p checker --target thumbv7em-none-eabihf
cargo check -p determinism --no-default-features --target thumbv7em-none-eabihf

echo "== tests"
cargo test --workspace

echo "== hazards cannot be reintroduced into kv-core"
scripts/hazard-check.sh

echo "== all checks passed"
