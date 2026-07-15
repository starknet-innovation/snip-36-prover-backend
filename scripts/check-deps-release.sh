#!/usr/bin/env bash
# Verify that a prebuilt deps release was produced from the expected source
# revisions and publishes every artifact consumed by this repository.

set -euo pipefail

if [ "$#" -lt 4 ] || [ "$#" -gt 5 ]; then
  echo "Usage: $0 <deps-vN> <sequencer-rev> <proving-utils-rev> <stwo-nightly> [owner/repo]" >&2
  exit 2
fi

tag="$1"
expected_sequencer="$2"
expected_proving_utils="$3"
expected_stwo_nightly="$4"
repo="${5:-${GITHUB_REPOSITORY:-starknet-innovation/snip-36-prover-backend}}"

case "$tag" in
  deps-v*) ;;
  *)
    echo "Error: dependency release must be a deps-v* tag, got: $tag" >&2
    exit 1
    ;;
esac

if ! command -v gh >/dev/null 2>&1; then
  echo "Error: gh is required to validate dependency release provenance" >&2
  exit 1
fi

body="$(gh release view "$tag" --repo "$repo" --json body --jq '.body // ""')"

release_value() {
  local label="$1"
  printf '%s\n' "$body" | awk -F'`' -v label="$label" '
    index($0, "- " label ":") == 1 { print $2; exit }
  '
}

check_value() {
  local label="$1"
  local expected="$2"
  local actual
  actual="$(release_value "$label")"
  if [ -z "$actual" ]; then
    echo "Error: dependency release $tag has no '$label' provenance field" >&2
    exit 1
  fi
  if [ "$actual" != "$expected" ]; then
    echo "Error: dependency release $tag has mismatched $label" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi
  echo "$label: $actual"
}

check_value "sequencer" "$expected_sequencer"
check_value "proving-utils" "$expected_proving_utils"
check_value "Rust nightly" "$expected_stwo_nightly"

assets="$(gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name')"
for asset in \
  snip36-deps-linux-x86_64.tar.gz \
  snip36-deps-linux-arm64.tar.gz \
  snip36-deps-darwin-arm64.tar.gz \
  SHA256SUMS; do
  if ! grep -Fxq "$asset" <<<"$assets"; then
    echo "Error: dependency release $tag is missing asset: $asset" >&2
    exit 1
  fi
done

echo "dependency release provenance verified: $tag"
