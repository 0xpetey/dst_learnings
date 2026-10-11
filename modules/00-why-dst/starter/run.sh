#!/usr/bin/env bash
# Run the binary N times (default 1000) and report how often it fails.
#
#   ./run.sh            # 1000 runs with the binary's default parameters
#   ./run.sh 200        # 200 runs
#   ARGS="4 2000 50 10" ./run.sh 200   # pass parameters through to the binary
#   VERBOSE=1 ./run.sh  # print the FAIL line of every failing run
#
# Exit code: 0 if no run failed, 1 otherwise, 2 if the binary is missing.
set -u
N="${1:-1000}"
BIN="${BIN:-target/release/lost_update}"
ARGS="${ARGS:-}"
if [ ! -x "$BIN" ]; then
  echo "binary $BIN not found; run: cargo build --release" >&2
  exit 2
fi
fail=0
for ((i = 1; i <= N; i++)); do
  # shellcheck disable=SC2086  # ARGS is meant to word-split
  if ! out="$("$BIN" $ARGS 2>/dev/null)"; then
    fail=$((fail + 1))
    if [ "${VERBOSE:-0}" = "1" ]; then
      echo "run $i: $out"
    fi
  fi
done
pct=$(awk -v f="$fail" -v n="$N" 'BEGIN { printf "%.1f", 100 * f / n }')
echo "$fail of $N runs failed ($pct%)"
[ "$fail" -eq 0 ]
