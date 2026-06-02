# syntax=docker/dockerfile:1
#
# All-in-one SNIP-36 prover image. Bundles the snip36 CLI + playground server
# and the prebuilt proving stack (stwo prover, virtual-OS runner, sierra
# compiler, bootloader) so proving + submitting run entirely in-container.
#
# Built by .github/workflows/build-deps.yml from the release's Linux artifacts —
# NOT meant to be built standalone (it expects the two tarballs in the context).
#
# Base is ubuntu:24.04 to match the glibc the release binaries are built against
# (GitHub's ubuntu-latest runner). Contract-dev tooling (scarb, sncast) and the
# Python cairo-compile venv are intentionally NOT included — they're for
# authoring/deploying contracts, not proving.

FROM ubuntu:24.04

# Runtime libs for the native prover/runner.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

ENV SNIP36_PROJECT_DIR=/app \
    PORT=8090
WORKDIR /app

# Application binaries (snip36 CLI + playground server) onto PATH. The tarball
# was created with `tar -C app .`, so it expands to snip36 + snip36-playground.
ADD snip36-linux-x86_64.tar.gz /usr/local/bin/

# Prebuilt proving stack, laid out exactly where snip36's Config expects it
# (deps_dir = $SNIP36_PROJECT_DIR/deps). Mirrors scripts/download-deps.sh.
ADD snip36-deps-linux-x86_64.tar.gz /tmp/snip36-deps/
RUN set -eux; \
    mkdir -p deps/bin deps/sequencer/target/release/shared_executables; \
    cp /tmp/snip36-deps/stwo-run-and-prove          deps/bin/; \
    cp /tmp/snip36-deps/bootloader_program.json     deps/bin/; \
    cp /tmp/snip36-deps/starknet_transaction_prover deps/sequencer/target/release/; \
    cp /tmp/snip36-deps/starknet_os_runner          deps/sequencer/target/release/; \
    cp /tmp/snip36-deps/shared_executables/bin/starknet-sierra-compile \
       deps/sequencer/target/release/shared_executables/; \
    chmod +x deps/bin/stwo-run-and-prove \
             deps/sequencer/target/release/starknet_transaction_prover \
             deps/sequencer/target/release/starknet_os_runner \
             deps/sequencer/target/release/shared_executables/starknet-sierra-compile; \
    rm -rf /tmp/snip36-deps

# Resource templates + the prove script the playground server shells out to.
COPY sample-input/ sample-input/
COPY scripts/ scripts/

EXPOSE 8090

# Required at runtime (pass with `-e`): STARKNET_RPC_URL, STARKNET_ACCOUNT_ADDRESS,
# STARKNET_PRIVATE_KEY, STARKNET_GATEWAY_URL. See README / .env.example.
#
# Default command runs the playground API server. Override to use the CLI, e.g.
#   docker run --rm -e STARKNET_RPC_URL=... <image> snip36 prove virtual-os ...
CMD ["snip36-playground"]
