#!/usr/bin/env bash
# Download pre-built SNIP-36 prover dependencies instead of building from source.
#
# Usage: ./scripts/download-deps.sh [RELEASE_TAG]
#
# This replaces `snip36 setup` and takes ~30 seconds instead of ~30 minutes.
# You still need Python 3.12 for cairo-compile (installed separately).

set -euo pipefail

REPO="${SNIP36_DEPS_REPO:-starknet-innovation/snip-36-prover-backend}"
TAG="${1:-deps-v1}"

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
URL="https://github.com/${REPO}/releases/download/${TAG}/snip36-deps-${PLATFORM}.tar.gz"

echo "=== SNIP-36 Dependency Download ==="
echo "Platform: ${PLATFORM}"
echo "Release:  ${TAG}"
echo "URL:      ${URL}"
echo ""

# Create directories
mkdir -p deps/bin
mkdir -p deps/sequencer/target/release

# Download and extract
echo "Downloading pre-built binaries..."
if command -v curl &>/dev/null; then
  curl -fSL "$URL" | tar xz -C deps/bin/
elif command -v wget &>/dev/null; then
  wget -qO- "$URL" | tar xz -C deps/bin/
else
  echo "Error: neither curl nor wget found"
  exit 1
fi

# Move starknet_os_runner to expected location
if [ -f deps/bin/starknet_os_runner ]; then
  mv deps/bin/starknet_os_runner deps/sequencer/target/release/starknet_os_runner
  chmod +x deps/sequencer/target/release/starknet_os_runner
fi

# Ensure executables are executable
chmod +x deps/bin/stwo-run-and-prove 2>/dev/null || true

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
  echo "  git clone --depth 1 -b PRIVACY-0.14.2-RC.2 https://github.com/starkware-libs/sequencer.git deps/sequencer"
  echo "  sequencer_venv/bin/pip install -r deps/sequencer/scripts/requirements.txt"
fi

echo ""
echo "=== Verification ==="
[ -f deps/bin/stwo-run-and-prove ] && echo "  stwo-run-and-prove: OK ($(du -h deps/bin/stwo-run-and-prove | cut -f1))" || echo "  stwo-run-and-prove: MISSING"
[ -f deps/sequencer/target/release/starknet_os_runner ] && echo "  starknet_os_runner: OK ($(du -h deps/sequencer/target/release/starknet_os_runner | cut -f1))" || echo "  starknet_os_runner: MISSING"
[ -f deps/bin/bootloader_program.json ] && echo "  bootloader_program: OK ($(du -h deps/bin/bootloader_program.json | cut -f1))" || echo "  bootloader_program: MISSING"
[ -f sequencer_venv/bin/cairo-compile ] && echo "  cairo-compile: OK" || echo "  cairo-compile: MISSING"

echo ""
echo "Done. You can now run: cargo run --release -p snip36-server"
