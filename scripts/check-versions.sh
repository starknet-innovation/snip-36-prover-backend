#!/usr/bin/env bash
# Single-source-of-truth guard for the app semver.
#
# The version lives ONCE in [workspace.package] in the root Cargo.toml. This
# script verifies the copies that cannot inherit it mechanically:
#   - extractor/Cargo.toml (excluded from the workspace, so it cannot use
#     version.workspace = true)
#   - the v* release tag, when given as $1 (tag v1.2.0 must equal "1.2.0")
#
# Run by ci.yml on every PR and by the build-deps.yml preflight job before
# release builds start.
#
# Usage: ./scripts/check-versions.sh [TAG]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

section_version() {
  # Print the first `version = "..."` inside the given TOML section.
  awk -F'"' -v section="$2" '
    $0 == "[" section "]" { in_section = 1; next }
    /^\[/                 { in_section = 0 }
    in_section && /^version[[:space:]]*=/ { print $2; exit }
  ' "$1"
}

ws_version="$(section_version "$ROOT/Cargo.toml" "workspace.package")"
extractor_version="$(section_version "$ROOT/extractor/Cargo.toml" "package")"

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
