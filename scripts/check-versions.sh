#!/usr/bin/env bash
# Single-source-of-truth guard for the app semver.
#
# The version lives ONCE in [workspace.package] in the root Cargo.toml. This
# script verifies the copies that cannot inherit it mechanically:
#   - extractor/Cargo.toml (excluded from the workspace, so it cannot use
#     version.workspace = true)
#   - the Scarb/Cairo version used by daily-health.yml and tests/contracts
#   - the v* release tag, when given as $1 (tag v1.2.0 must equal "1.2.0")
#
# Run by ci.yml on every PR and by the build-deps.yml preflight job before
# release builds start.
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

ws_version="$(toml_section_value "$ROOT/Cargo.toml" "workspace.package" "version")"
extractor_version="$(toml_section_value "$ROOT/extractor/Cargo.toml" "package" "version")"
contracts_cairo_version="$(toml_section_value "$ROOT/tests/contracts/Scarb.toml" "package" "cairo-version")"
daily_scarb_version="$(yaml_value "$ROOT/.github/workflows/daily-health.yml" "SCARB_VERSION")"

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
      echo "tag '$tag' is not a v* app tag; skipping tag/version check"
      ;;
  esac
fi

exit "$fail"
