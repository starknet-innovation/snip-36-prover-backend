#!/usr/bin/env bash
# Review regressions on the exact patched V2 source; no RPC, keys or transactions.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROVING="$(cd "${1:-$ROOT/deps/low-memory-prover/proving-v2}" && pwd)"
REV=e0035ffc8e08da270b8cb4a71f3708af70c2c775
test "$(git -C "$PROVING" rev-parse HEAD)" = "$REV"
git -C "$PROVING" apply --reverse --check "$ROOT/vendor/low-memory-prover/proving-v2.patch"
TARGET="${CARGO_TARGET_DIR:-$ROOT/deps/low-memory-v2-safety-target}"
CARGO=(cargo +nightly-2026-01-15)
ARGS=(--manifest-path "$PROVING/Cargo.toml" --target-dir "$TARGET")
"${CARGO[@]}" test "${ARGS[@]}" -p stwo --features prover,parallel --release --lib
"${CARGO[@]}" test "${ARGS[@]}" -p stwo --features prover --release --lib
"${CARGO[@]}" test "${ARGS[@]}" -p stwo --features prover --doc Vectorized
"${CARGO[@]}" test "${ARGS[@]}" -p stwo-constraint-framework --features prover,parallel --release --lib
"${CARGO[@]}" check "${ARGS[@]}" -p stwo-examples --features parallel
if [ "${SNIP36_MIRI:-0}" = 1 ]; then
  "${CARGO[@]}" miri test "${ARGS[@]}" -p stwo --features prover --lib column::safety_review_tests
  "${CARGO[@]}" miri test "${ARGS[@]}" -p stwo --features prover --lib mempool::safety_review_tests
fi
