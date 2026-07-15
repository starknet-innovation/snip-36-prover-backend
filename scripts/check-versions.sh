#!/usr/bin/env bash
# Single-source-of-truth guards for app and dependency versions.
#
# The version lives ONCE in [workspace.package] in the root Cargo.toml. This
# script verifies the copies that cannot inherit it mechanically:
#   - extractor/Cargo.toml (excluded from the workspace, so it cannot use
#     version.workspace = true)
#   - the Scarb/Cairo version used by daily-health.yml and tests/contracts
#   - dependency pins duplicated by the build workflow, daily E2E workflow,
#     and Rust source
#   - the v* release tag, when given as $1 (tag v1.2.0 must equal "1.2.0")
#
# Run by ci.yml on every PR, by daily-health.yml before E2E provisioning, and
# by the build-deps.yml preflight job before release builds start.
#
# Usage: ./scripts/check-versions.sh [TAG]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

toml_section_value() {
  # Print the first string value for a key inside the given TOML section.
  awk -F'"' -v section="$2" -v key="$3" '
    $0 == "[" section "]" { in_section = 1; next }
    /^\[/                 { in_section = 0 }
    in_section && $0 ~ "^" key "[[:space:]]*=" { print $2; exit }
  ' "$1"
}

yaml_value() {
  awk -v key="$2" '
    $0 ~ "^[[:space:]]*" key ":" {
      value = $0
      sub(/^[[:space:]]*[^:]+:[[:space:]]*/, "", value)
      sub(/[[:space:]]*#.*/, "", value)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
      gsub(/^"|"$/, "", value)
      print value
      exit
    }
  ' "$1"
}

rust_const_value() {
  awk -F'"' -v key="$2" '
    $0 ~ "^const " key ":[[:space:]]*&str[[:space:]]*=" { print $2; exit }
  ' "$1"
}

ws_version="$(toml_section_value "$ROOT/Cargo.toml" "workspace.package" "version")"
extractor_version="$(toml_section_value "$ROOT/extractor/Cargo.toml" "package" "version")"
contracts_cairo_version="$(toml_section_value "$ROOT/tests/contracts/Scarb.toml" "package" "cairo-version")"
daily_scarb_version="$(yaml_value "$ROOT/.github/workflows/daily-health.yml" "SCARB_VERSION")"
deps_version="$(tr -d '[:space:]' < "$ROOT/deps-version")"

build_sequencer="$(yaml_value "$ROOT/.github/workflows/build-deps.yml" "SEQUENCER_TAG")"
build_proving_utils="$(yaml_value "$ROOT/.github/workflows/build-deps.yml" "PROVING_UTILS_REV")"
build_stwo_nightly="$(yaml_value "$ROOT/.github/workflows/build-deps.yml" "STWO_NIGHTLY")"

daily_sequencer="$(yaml_value "$ROOT/.github/workflows/daily-health.yml" "SEQUENCER_TAG")"
daily_proving_utils="$(yaml_value "$ROOT/.github/workflows/daily-health.yml" "PROVING_UTILS_REV")"
daily_stwo_nightly="$(yaml_value "$ROOT/.github/workflows/daily-health.yml" "STWO_NIGHTLY")"

setup_rs="$ROOT/crates/snip36-cli/src/commands/setup.rs"
rust_sequencer="$(rust_const_value "$setup_rs" "SEQUENCER_TAG")"
rust_proving_utils="$(rust_const_value "$setup_rs" "PROVING_UTILS_VERSION")"
rust_stwo_nightly="$(rust_const_value "$setup_rs" "STWO_NIGHTLY")"

if [ -z "$ws_version" ]; then
  echo "Error: could not parse [workspace.package] version from Cargo.toml" >&2
  exit 1
fi
echo "workspace version: $ws_version"

fail=0
if [ "$extractor_version" != "$ws_version" ]; then
  echo "Error: extractor/Cargo.toml version ($extractor_version) != workspace version ($ws_version)" >&2
  fail=1
fi

if [ "$daily_scarb_version" != "$contracts_cairo_version" ]; then
  echo "Error: daily-health.yml SCARB_VERSION ($daily_scarb_version) != tests/contracts/Scarb.toml cairo-version ($contracts_cairo_version)" >&2
  fail=1
fi

case "$deps_version" in
  deps-v*)
    deps_number="${deps_version#deps-v}"
    case "$deps_number" in
      ''|*[!0-9]*)
        echo "Error: deps-version must contain a deps-v<n> tag, got: $deps_version" >&2
        fail=1
        ;;
    esac
    ;;
  *)
    echo "Error: deps-version must contain a deps-v<n> tag, got: $deps_version" >&2
    fail=1
    ;;
esac

check_pin_sync() {
  label="$1"
  build_value="$2"
  daily_value="$3"
  rust_value="$4"
  if [ -z "$build_value" ] || [ -z "$daily_value" ] || [ -z "$rust_value" ]; then
    echo "Error: could not parse every $label pin" >&2
    fail=1
  elif [ "$build_value" != "$daily_value" ] || [ "$build_value" != "$rust_value" ]; then
    echo "Error: $label pin mismatch" >&2
    echo "  build-deps.yml:  $build_value" >&2
    echo "  daily-health.yml: $daily_value" >&2
    echo "  setup.rs:         $rust_value" >&2
    fail=1
  else
    echo "$label pin: $build_value"
  fi
}

check_pin_sync "sequencer" "$build_sequencer" "$daily_sequencer" "$rust_sequencer"
check_pin_sync "proving-utils" "$build_proving_utils" "$daily_proving_utils" "$rust_proving_utils"
check_pin_sync "Rust nightly" "$build_stwo_nightly" "$daily_stwo_nightly" "$rust_stwo_nightly"

if [ $# -ge 1 ]; then
  tag="$1"
  case "$tag" in
    v*)
      if [ "${tag#v}" != "$ws_version" ]; then
        echo "Error: release tag $tag != workspace version $ws_version." >&2
        echo "Bump [workspace.package] version in Cargo.toml first (see RELEASING.md)." >&2
        fail=1
      else
        echo "release tag matches: $tag"
      fi
      ;;
    *)
      echo "tag '$tag' is not a v* app tag; skipping app tag/version check"
      ;;
  esac
fi

exit "$fail"
