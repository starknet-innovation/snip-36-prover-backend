#!/usr/bin/env bash
# Offline smoke test for the packaged prover deps: compiles a tiny fixture task,
# proves it through the bundled bootloader with stwo-run-and-prove, and verifies
# the proof in-process (--verify). No network, no secrets, no chain state.
#
# Usage: ./scripts/smoke-test-deps.sh <dist-dir>
#   <dist-dir> must contain stwo-run-and-prove and bootloader_program.json
#   (the layout produced by the "Package dependency binaries" step).
#
# Env overrides:
#   SNIP36_VENV — venv providing cairo-compile (default: <repo>/sequencer_venv).
#     Note: only cairo-compile is needed; cairo-run is NOT used (it is broken on
#     Python >= 3.11 with cairo-lang 0.14.x due to a dataclasses change).

set -euo pipefail

DIST_DIR="$(cd "${1:?usage: smoke-test-deps.sh <dist-dir>}" && pwd)"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENV="${SNIP36_VENV:-$ROOT/sequencer_venv}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

test -x "$DIST_DIR/stwo-run-and-prove" || { echo "missing $DIST_DIR/stwo-run-and-prove" >&2; exit 1; }
test -f "$DIST_DIR/bootloader_program.json" || { echo "missing $DIST_DIR/bootloader_program.json" >&2; exit 1; }
test -x "$VENV/bin/cairo-compile" || { echo "missing $VENV/bin/cairo-compile (run snip36 setup, or set SNIP36_VENV)" >&2; exit 1; }

echo "=== smoke: compiling fixture task ==="
"$VENV/bin/cairo-compile" "$ROOT/sample-input/smoke_task.cairo" \
  --output "$WORK/smoke_task.json"

# SimpleBootloaderInput (flat schema — see sample-input/README.md). poseidon task
# hashing keeps the proof compatible with the default prover params
# (preprocessed_trace: canonical_without_pedersen).
cat > "$WORK/bootloader_input.json" <<EOF
{
  "tasks": [
    {
      "path": "$WORK/smoke_task.json",
      "program_hash_function": "poseidon",
      "type": "RunProgramTask"
    }
  ],
  "single_page": true
}
EOF

echo "=== smoke: prove + verify through bundled bootloader ==="
"$DIST_DIR/stwo-run-and-prove" \
  --program "$DIST_DIR/bootloader_program.json" \
  --program_input "$WORK/bootloader_input.json" \
  --prover_params_json "$ROOT/sample-input/prover_params.json" \
  --proof_path "$WORK/smoke.proof" \
  --proof-format binary \
  --verify

test -s "$WORK/smoke.proof" || { echo "proof file missing or empty" >&2; exit 1; }
echo "=== smoke: OK — proof generated and verified ($(wc -c < "$WORK/smoke.proof" | tr -d ' ') bytes) ==="
