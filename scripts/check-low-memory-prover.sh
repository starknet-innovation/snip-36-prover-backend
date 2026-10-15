#!/usr/bin/env bash
# Check the low-memory prover (vendor/low-memory-prover) on this machine: prove the
# fixture STRK20 PIEs with the patched privacy_prove, require the exact proof bytes
# the unpatched prover produces, and report peak memory.
#
# usage: scripts/check-low-memory-prover.sh [deps-dir]
# Run `snip36 setup` (source build) first: it writes the patched trees and the
# [patch] section this script reuses from deps/sequencer/Cargo.toml.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEPS="${1:-$ROOT/deps}"
CHECK="$ROOT/vendor/low-memory-prover/check"
NIGHTLY="nightly-2026-01-15"
MARKER="# snip36: low-memory prover"

grep -q "$MARKER" "$DEPS/sequencer/Cargo.toml" 2>/dev/null || {
  echo "error: $DEPS/sequencer/Cargo.toml has no low-memory prover section; run snip36 setup first" >&2
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp -R "$CHECK/." "$WORK/"
sed -n "/^$MARKER/,\$p" "$DEPS/sequencer/Cargo.toml" >> "$WORK/Cargo.toml"

echo "=== building the check against $DEPS/low-memory-prover"
cargo "+$NIGHTLY" build --release --locked --quiet --manifest-path "$WORK/Cargo.toml" \
  --target-dir "${CARGO_TARGET_DIR:-$DEPS/low-memory-prover/target}"
BIN="${CARGO_TARGET_DIR:-$DEPS/low-memory-prover/target}/release/low-memory-check"

peak() { # prints peak RSS in GB from a time(1) log
  if grep -q "Maximum resident set size" "$1"; then # GNU time -v (kbytes)
    awk -F: '/Maximum resident set size/{printf "%.2f GB", $2/1e6}' "$1"
  else                                               # BSD time -l (bytes)
    awk '/maximum resident set size/{printf "%.2f GB", $1/1e9}' "$1"
  fi
}
if /usr/bin/time -v true >/dev/null 2>&1; then TIME=(/usr/bin/time -v); else TIME=(/usr/bin/time -l); fi

fail=0
while read -r want name; do
  "${TIME[@]}" "$BIN" "$CHECK/fixtures/$name.pie.zip" "$WORK/$name.proof" 2> "$WORK/$name.log"
  got="$(shasum -a 256 "$WORK/$name.proof" | cut -d' ' -f1)"
  if [ "$got" = "$want" ]; then verdict="OK (byte-identical)"; else verdict="MISMATCH ($got)"; fail=1; fi
  echo "$name: $(grep -o 'prove [0-9.]*s' "$WORK/$name.log"), peak $(peak "$WORK/$name.log") — $verdict"
done < "$CHECK/fixtures/expected.sha256"
exit "$fail"
