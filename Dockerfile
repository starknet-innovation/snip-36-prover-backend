# syntax=docker/dockerfile:1
#
# All-in-one SNIP-36 prover image. Bundles the snip36 CLI and the prebuilt
# proving stack (stwo prover, virtual-OS runner, sierra compiler, bootloader) so
# proving + submitting run entirely in-container.
#
# Built by .github/workflows/build-deps.yml from the release's Linux artifacts —
# NOT meant to be built standalone. The workflow stages the building platform's
# tarballs into the context under the fixed names snip36.tar.gz and
# snip36-deps.tar.gz (ADD cannot interpolate a per-arch source from a build
# arg), so the same Dockerfile produces both linux/amd64 and linux/arm64.
#
# Base is ubuntu:24.04 (multi-arch) to match the glibc the release binaries are
# built against (GitHub's ubuntu-latest / ubuntu-24.04-arm runners).
# Contract-dev tooling (scarb, sncast) and the Python cairo-compile venv are
# intentionally NOT included — they're for authoring/deploying contracts, not
# proving.

FROM ubuntu:24.04

# The prebuilt starknet_transaction_prover embeds the GitHub Actions workspace
# path through the sequencer's RUNTIME_ACCESSIBLE_OUT_DIR build-time env. Keep
# this default in sync with the official repository's runner workspace; the
# release workflow passes the actual workspace path as a build arg.
ARG RUNNER_BUILD_WORKSPACE=/home/runner/work/snip-36-prover-backend/snip-36-prover-backend

# Runtime libs for the native prover/runner.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

ENV SNIP36_PROJECT_DIR=/app
WORKDIR /app

# Install the snip36 CLI onto PATH. The tarball expands to snip36 +
# snip36-playground; keep only the CLI (the playground server is not included).
ADD snip36.tar.gz /tmp/snip36-app/
RUN cp /tmp/snip36-app/snip36 /usr/local/bin/snip36 \
    && chmod +x /usr/local/bin/snip36 \
    && rm -rf /tmp/snip36-app

# Prebuilt proving stack, laid out exactly where snip36's Config expects it
# (deps_dir = $SNIP36_PROJECT_DIR/deps). Mirrors scripts/download-deps.sh.
ADD snip36-deps.tar.gz /tmp/snip36-deps/
RUN set -eux; \
    mkdir -p deps/bin deps/sequencer/target/release/shared_executables; \
    cp /tmp/snip36-deps/stwo-run-and-prove          deps/bin/; \
    cp /tmp/snip36-deps/bootloader_program.json     deps/bin/; \
    cp /tmp/snip36-deps/starknet_transaction_prover deps/sequencer/target/release/; \
    cp /tmp/snip36-deps/starknet_os_runner          deps/sequencer/target/release/; \
    # deps-v4+ tarballs ship the sierra compiler flat at shared_executables/;
    # older tags nest it under shared_executables/bin/. Accept both so the
    # image can still be rebuilt from older release tarballs.
    if [ -f /tmp/snip36-deps/shared_executables/starknet-sierra-compile ]; then \
      cp /tmp/snip36-deps/shared_executables/starknet-sierra-compile \
         deps/sequencer/target/release/shared_executables/; \
    else \
      cp /tmp/snip36-deps/shared_executables/bin/starknet-sierra-compile \
         deps/sequencer/target/release/shared_executables/; \
    fi; \
    chmod +x deps/bin/stwo-run-and-prove \
             deps/sequencer/target/release/starknet_transaction_prover \
             deps/sequencer/target/release/starknet_os_runner \
             deps/sequencer/target/release/shared_executables/starknet-sierra-compile; \
    if [ "$RUNNER_BUILD_WORKSPACE" != "$SNIP36_PROJECT_DIR" ]; then \
      mkdir -p "$RUNNER_BUILD_WORKSPACE/deps"; \
      rm -rf "$RUNNER_BUILD_WORKSPACE/deps/sequencer"; \
      ln -s "$SNIP36_PROJECT_DIR/deps/sequencer" "$RUNNER_BUILD_WORKSPACE/deps/sequencer"; \
      test -x "$RUNNER_BUILD_WORKSPACE/deps/sequencer/target/release/shared_executables/starknet-sierra-compile"; \
    fi; \
    rm -rf /tmp/snip36-deps

# Prover-parameter templates used by the `prove program` / `prove pie` paths.
COPY sample-input/ sample-input/

# Required at runtime (pass with `-e`): STARKNET_RPC_URL, STARKNET_ACCOUNT_ADDRESS,
# STARKNET_PRIVATE_KEY, STARKNET_GATEWAY_URL. See README / .env.example.
#
# The image IS the snip36 CLI; args pass straight through, e.g.
#   docker run --rm -e STARKNET_RPC_URL=... <image> prove virtual-os --tx-hash 0x... ...
ENTRYPOINT ["snip36"]
CMD ["--help"]
