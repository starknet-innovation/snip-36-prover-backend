#!/usr/bin/env bash
# Download pre-built SNIP-36 prover dependencies instead of building from source.
#
# Usage: ./scripts/download-deps.sh [RELEASE_TAG]
#
# This replaces `snip36 setup` and takes ~30 seconds instead of ~30 minutes.
# You still need Python 3.12 for cairo-compile (installed separately).

set -euo pipefail

REPO="${SNIP36_DEPS_REPO:-starknet-innovation/snip-36-prover-backend}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Default deps release: the deps-version file at the repo root is the single
# source of truth (also baked into snip36 by crates/snip36-cli/build.rs and
# read by daily-health.yml). Bump it when cutting a new deps-v* (RELEASING.md).
TAG="${1:-$(cat "$SCRIPT_DIR/../deps-version")}"

# Detect platform
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) ARCH="arm64" ;;
  x86_64)        ARCH="x86_64" ;;
  *)
    echo "Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

PLATFORM="${OS}-${ARCH}"

# Prebuilt assets exist only for these platforms — keep in sync with the
# build matrix in .github/workflows/build-deps.yml.
case "$PLATFORM" in
  linux-x86_64|darwin-arm64) ;;
  *)
    echo "Error: no prebuilt deps for ${PLATFORM}." >&2
    echo "Supported platforms: linux-x86_64, darwin-arm64." >&2
    echo "Build from source instead: cargo build --release -p snip36-cli && snip36 setup" >&2
    exit 1
    ;;
esac

URL="https://github.com/${REPO}/releases/download/${TAG}/snip36-deps-${PLATFORM}.tar.gz"

echo "=== SNIP-36 Dependency Download ==="
echo "Platform: ${PLATFORM}"
echo "Release:  ${TAG}"
echo "URL:      ${URL}"
echo ""

# Create directories
mkdir -p deps/bin
mkdir -p deps/sequencer/target/release

# Download to a temp file (not a pipe) so the checksum can be verified
# before extraction.
SUMS_URL="https://github.com/${REPO}/releases/download/${TAG}/SHA256SUMS"
TMP_TAR="$(mktemp)"
trap 'rm -f "$TMP_TAR"' EXIT

echo "Downloading pre-built binaries..."
if command -v curl &>/dev/null; then
  curl -fSL "$URL" -o "$TMP_TAR"
  SUMS="$(curl -fsSL "$SUMS_URL" 2>/dev/null || true)"
elif command -v wget &>/dev/null; then
  wget -qO "$TMP_TAR" "$URL"
  SUMS="$(wget -qO- "$SUMS_URL" 2>/dev/null || true)"
else
  echo "Error: neither curl nor wget found"
  exit 1
fi

# Verify against SHA256SUMS when the release publishes one (deps-v4+); warn
# rather than fail on older tags that predate checksums.
if [ -n "$SUMS" ]; then
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "$TMP_TAR" | awk '{print $1}')"
  else
    ACTUAL="$(shasum -a 256 "$TMP_TAR" | awk '{print $1}')"
  fi
  EXPECTED="$(printf '%s\n' "$SUMS" | awk -v f="snip36-deps-${PLATFORM}.tar.gz" '$2 == f {print $1}')"
  if [ -z "$EXPECTED" ]; then
    echo "WARNING: ${TAG} SHA256SUMS has no entry for snip36-deps-${PLATFORM}.tar.gz; skipping verification"
  elif [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "Error: checksum mismatch for snip36-deps-${PLATFORM}.tar.gz" >&2
    echo "  expected: $EXPECTED" >&2
    echo "  actual:   $ACTUAL" >&2
    exit 1
  else
    echo "Checksum verified."
  fi
else
  echo "WARNING: no SHA256SUMS published for ${TAG}; skipping checksum verification"
fi

tar xz -C deps/bin/ -f "$TMP_TAR"

# Move runner binaries to their expected locations. The current CLI expects
# starknet_transaction_prover; starknet_os_runner is kept as a compatibility
# alias for older scripts.
if [ -f deps/bin/starknet_transaction_prover ]; then
  mv deps/bin/starknet_transaction_prover deps/sequencer/target/release/starknet_transaction_prover
  chmod +x deps/sequencer/target/release/starknet_transaction_prover
fi

if [ -f deps/bin/starknet_os_runner ]; then
  mv deps/bin/starknet_os_runner deps/sequencer/target/release/starknet_os_runner
  chmod +x deps/sequencer/target/release/starknet_os_runner
fi

if [ -f deps/sequencer/target/release/starknet_transaction_prover ] && [ ! -f deps/sequencer/target/release/starknet_os_runner ]; then
  cp deps/sequencer/target/release/starknet_transaction_prover deps/sequencer/target/release/starknet_os_runner
  chmod +x deps/sequencer/target/release/starknet_os_runner
fi

if [ -f deps/sequencer/target/release/starknet_os_runner ] && [ ! -f deps/sequencer/target/release/starknet_transaction_prover ]; then
  cp deps/sequencer/target/release/starknet_os_runner deps/sequencer/target/release/starknet_transaction_prover
  chmod +x deps/sequencer/target/release/starknet_transaction_prover
fi

# Move starknet-sierra-compile to the sequencer target location expected by
# sequencer tooling. deps-v4+ tarballs ship it flat at
# shared_executables/starknet-sierra-compile; older tags (<= deps-v3) nest it
# under shared_executables/bin/. Accept both.
for sierra_src in \
  deps/bin/shared_executables/starknet-sierra-compile \
  deps/bin/shared_executables/bin/starknet-sierra-compile; do
  if [ -f "$sierra_src" ]; then
    mkdir -p deps/sequencer/target/release/shared_executables
    mv "$sierra_src" deps/sequencer/target/release/shared_executables/starknet-sierra-compile
    chmod +x deps/sequencer/target/release/shared_executables/starknet-sierra-compile
    break
  fi
done
rm -rf deps/bin/shared_executables 2>/dev/null || true

# Ensure executables are executable
chmod +x deps/bin/stwo-run-and-prove 2>/dev/null || true

# Stamp the provisioned release so snip36 can warn on a deps/CLI mismatch.
echo "$TAG" > deps/.deps-version

# Clear the macOS Gatekeeper quarantine attribute if present. curl/wget
# downloads are not quarantined, but tarballs fetched via a browser and
# extracted here would be — stripping is idempotent and harmless otherwise.
if [ "$OS" = "darwin" ]; then
  xattr -dr com.apple.quarantine deps/ 2>/dev/null || true
fi

# Set up Python venv for cairo-compile (still needed)
echo ""
echo "Setting up Python venv for cairo-compile..."

PYTHON_BIN="python3.12"
if ! command -v "$PYTHON_BIN" &>/dev/null; then
  PYTHON_BIN="python3"
fi

if [ ! -f sequencer_venv/bin/pip ]; then
  "$PYTHON_BIN" -m venv sequencer_venv
fi

# Install cairo-lang requirements if sequencer repo is available
if [ -f deps/sequencer/scripts/requirements.txt ]; then
  sequencer_venv/bin/pip install --quiet -r deps/sequencer/scripts/requirements.txt
  echo "cairo-compile installed"
elif [ -f sequencer_venv/bin/cairo-compile ]; then
  echo "cairo-compile already available"
else
  echo "WARNING: sequencer repo not cloned — cairo-compile not installed."
  echo "You may need to clone the sequencer for the Python venv:"
  echo "  git clone --depth 1 -b PRIVACY-0.14.2-RC.6 https://github.com/starkware-libs/sequencer.git deps/sequencer"
  echo "  sequencer_venv/bin/pip install -r deps/sequencer/scripts/requirements.txt"
fi

echo ""
echo "=== Verification ==="
[ -f deps/bin/stwo-run-and-prove ] && echo "  stwo-run-and-prove: OK ($(du -h deps/bin/stwo-run-and-prove | cut -f1))" || echo "  stwo-run-and-prove: MISSING"
[ -f deps/sequencer/target/release/starknet_transaction_prover ] && echo "  starknet_transaction_prover: OK ($(du -h deps/sequencer/target/release/starknet_transaction_prover | cut -f1))" || echo "  starknet_transaction_prover: MISSING"
[ -f deps/sequencer/target/release/starknet_os_runner ] && echo "  starknet_os_runner: OK ($(du -h deps/sequencer/target/release/starknet_os_runner | cut -f1))" || echo "  starknet_os_runner: MISSING"
[ -f deps/sequencer/target/release/shared_executables/starknet-sierra-compile ] && echo "  starknet-sierra-compile: OK ($(du -h deps/sequencer/target/release/shared_executables/starknet-sierra-compile | cut -f1))" || echo "  starknet-sierra-compile: MISSING"
[ -f deps/bin/bootloader_program.json ] && echo "  bootloader_program: OK ($(du -h deps/bin/bootloader_program.json | cut -f1))" || echo "  bootloader_program: MISSING"
[ -f sequencer_venv/bin/cairo-compile ] && echo "  cairo-compile: OK" || echo "  cairo-compile: MISSING"

echo ""
echo "Done. You can now run: cargo run --release -p snip36-server"
