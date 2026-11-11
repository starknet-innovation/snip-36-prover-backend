#!/usr/bin/env bash
# Prove/verify the V2 fixtures against a pinned checkout, and require stock proof bytes.
# Usage: scripts/check-low-memory-prover-v2.sh [proving-checkout] [repeats]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROVING="$(cd "${1:-$ROOT/deps/low-memory-prover/proving-v2}" && pwd)"
REV=e0035ffc8e08da270b8cb4a71f3708af70c2c775
[ "$(git -C "$PROVING" rev-parse HEAD)" = "$REV" ] || { echo "unsupported V2 checkout" >&2; exit 1; }
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
CHECK="$ROOT/vendor/low-memory-prover/check-v2"
cp -R "$CHECK/." "$WORK/"
python3 - "$WORK" "$PROVING" <<'PY'
import json,sys
from pathlib import Path
work,proving=map(Path,sys.argv[1:])
s=(work/'Cargo.toml.in').read_text()
s=s.replace('@PRIVACY_PROVE@',json.dumps(str(proving/'crates/privacy_prove')))
s=s.replace('@PRIVACY_VERIFY@',json.dumps(str(proving/'crates/privacy_circuit_verify')))
(work/'Cargo.toml').write_text(s)
PY
TARGET="${CARGO_TARGET_DIR:-$ROOT/deps/low-memory-v2-target}"
cargo +nightly-2026-01-15 build --release --locked --manifest-path "$WORK/Cargo.toml" --target-dir "$TARGET"
if /usr/bin/time -v true >/dev/null 2>&1; then TIME=(/usr/bin/time -v); else TIME=(/usr/bin/time -l); fi
while read -r want name; do
  if ! "${TIME[@]}" "$TARGET/release/low-memory-v2-check" "$ROOT/vendor/low-memory-prover/check/fixtures/$name.pie.zip" "$WORK/$name.proof" "${2:-1}" 2> "$WORK/$name.log"; then
    cat "$WORK/$name.log" >&2
    exit 1
  fi
  cat "$WORK/$name.log"
  got="$(shasum -a 256 "$WORK/$name.proof" | cut -d' ' -f1)"
  [ "$got" = "$want" ] || { echo "$name: V2 proof mismatch ($got)" >&2; exit 1; }
  echo "$name: verified and byte-identical to stock V2 ($got)"
done < "$CHECK/expected.sha256"
