#!/usr/bin/env sh
# One-line installer for the snip36 CLI:
#
#   curl -fsSL https://github.com/starknet-innovation/snip-36-prover-backend/releases/latest/download/install.sh | sh
#
# Installs snip36 + snip36-playground from the latest v* GitHub release into
# ~/.local/bin (override with SNIP36_INSTALL_DIR). Pass a tag as $1 to pin a
# specific release. Supported platforms: linux-x86_64, linux-arm64,
# darwin-arm64.
#
# After installing, fetch the proving stack in your project directory with:
#   snip36 setup --prebuilt

set -eu

REPO="${SNIP36_DEPS_REPO:-starknet-innovation/snip-36-prover-backend}"
INSTALL_DIR="${SNIP36_INSTALL_DIR:-$HOME/.local/bin}"
TAG="${1:-latest}"

# Detect platform (keep in sync with the build matrix in build-deps.yml)
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) ARCH="arm64" ;;
  x86_64)        ARCH="x86_64" ;;
  *)
    echo "Error: unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac
PLATFORM="${OS}-${ARCH}"

case "$PLATFORM" in
  linux-x86_64|linux-arm64|darwin-arm64) ;;
  *)
    echo "Error: no prebuilt snip36 for ${PLATFORM}." >&2
    echo "Supported platforms: linux-x86_64, linux-arm64, darwin-arm64." >&2
    echo "Build from source instead: cargo build --release -p snip36-cli" >&2
    exit 1
    ;;
esac

if [ "$TAG" = "latest" ]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
fi
ASSET="snip36-${PLATFORM}.tar.gz"

echo "=== snip36 installer ==="
echo "Platform: ${PLATFORM}"
echo "Release:  ${TAG}"
echo "Target:   ${INSTALL_DIR}"
echo ""

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading ${ASSET}..."
curl -fSL "${BASE_URL}/${ASSET}" -o "${TMP}/${ASSET}"

# Verify against SHA256SUMS when the release publishes one (v1.3.0+).
if curl -fsSL "${BASE_URL}/SHA256SUMS" -o "${TMP}/SHA256SUMS" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    SHA_TOOL="sha256sum"
  else
    SHA_TOOL="shasum -a 256"
  fi
  (cd "$TMP" && grep "  ${ASSET}\$" SHA256SUMS | $SHA_TOOL -c -) \
    || { echo "Error: checksum verification failed for ${ASSET}" >&2; exit 1; }
else
  echo "WARNING: no SHA256SUMS published for this release; skipping checksum verification"
fi

tar xzf "${TMP}/${ASSET}" -C "$TMP"
mkdir -p "$INSTALL_DIR"
install -m 0755 "${TMP}/snip36" "${INSTALL_DIR}/snip36"
install -m 0755 "${TMP}/snip36-playground" "${INSTALL_DIR}/snip36-playground"

# Clear the macOS Gatekeeper quarantine attribute if present (set by
# browser-downloaded assets; curl downloads are not quarantined).
if [ "$OS" = "darwin" ]; then
  xattr -d com.apple.quarantine "${INSTALL_DIR}/snip36" "${INSTALL_DIR}/snip36-playground" 2>/dev/null || true
fi

echo ""
echo "Installed snip36 to ${INSTALL_DIR}/snip36"
case ":$PATH:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo "NOTE: ${INSTALL_DIR} is not on your PATH. Add it with:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac
echo ""
echo "Next: in your project directory, fetch the proving stack with:"
echo "  snip36 setup --prebuilt"
