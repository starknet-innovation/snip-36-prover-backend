# syntax=docker/dockerfile:1
#
# All-in-one SNIP-36 prover image. Bundles the snip36 CLI and the prebuilt
# proving stack (stwo prover, virtual-OS runner, sierra compiler, bootloader) so
# proving + submitting run entirely in-container.
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

ENV SNIP36_PROJECT_DIR=/app
WORKDIR /app

# Install the snip36 CLI onto PATH. The tarball expands to snip36 +
# snip36-playground; keep only the CLI (the playground server is not included).
ADD snip36-linux-x86_64.tar.gz /tmp/snip36-app/
RUN cp /tmp/snip36-app/snip36 /usr/local/bin/snip36 \
    && chmod +x /usr/local/bin/snip36 \
    && rm -rf /tmp/snip36-app

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

# Prover-parameter templates used by the `prove program` / `prove pie` paths.
COPY sample-input/ sample-input/

# Required at runtime (pass with `-e`): STARKNET_RPC_URL, STARKNET_ACCOUNT_ADDRESS,
# STARKNET_PRIVATE_KEY, STARKNET_GATEWAY_URL. See README / .env.example.
#
# The image IS the snip36 CLI; args pass straight through, e.g.
#   docker run --rm -e STARKNET_RPC_URL=... <image> prove virtual-os --tx-hash 0x... ...
ENTRYPOINT ["snip36"]
CMD ["--help"]
