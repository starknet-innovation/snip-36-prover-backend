# Releasing

This repo publishes GitHub Releases via `.github/workflows/build-deps.yml`,
which builds for **linux-x86_64**, **linux-arm64**, and **darwin-arm64**.
There are two independent tag schemes — pick the right one.

## Versioning

The crate version is defined **once**, in `[workspace.package]` in the root
`Cargo.toml`. All seven workspace members inherit it via
`version.workspace = true`; never set a per-crate version. The `extractor`
crate is excluded from the workspace (it needs `deps/sequencer/`) and carries
its own `version` — keep it in step manually. The two web apps
(`web/frontend`, `web/coinflip`) are npm packages versioned independently and
are not part of these releases.

**Rule:** the workspace version must equal the `v*` tag you release. `v1.2.0`
⇒ `version = "1.2.0"`.

Both rules are CI-enforced by `scripts/check-versions.sh`: every PR checks
the extractor sync (`ci.yml`), and the `build-deps.yml` preflight job fails a
`v*` release before the builds start if the tag doesn't match.

## `v<x.y.z>` — application release

Publishes the `snip36` CLI + `snip36-playground` binaries **and** the matching
prebuilt deps, for all three platforms, plus a `SHA256SUMS` file (7 assets
total).

1. Bump `version` in `[workspace.package]` (and `extractor/Cargo.toml`).
2. `cargo build --workspace` to refresh `Cargo.lock`.
3. Commit, open a PR, merge to `main` (the `CI / build & test` check must pass).
4. Tag the merge commit and push:
   ```bash
   git tag v1.2.0 && git push origin v1.2.0
   ```
5. `build-deps.yml` runs (~30–40 min) and:
   - creates the GitHub release with `snip36-<platform>.tar.gz` (`snip36` +
     `snip36-playground`) and `snip36-deps-<platform>.tar.gz` (prebuilt deps);
   - builds, smoke-tests (natively per arch), and pushes the all-in-one
     `snip36` CLI image (CLI + proving stack; no playground server) to
     `ghcr.io/starknet-innovation/snip-36-prover-backend:<x.y.z>` and `:latest`
     as a multi-arch manifest (linux/amd64 + linux/arm64 — the latter is what
     Docker on Apple Silicon pulls; see `Dockerfile`). The image is **not**
     published for `deps-v*` tags.

## `deps-v<n>` — prebuilt dependency bundle

Just the external prebuilt binaries (`stwo-run-and-prove`,
`starknet_transaction_prover` / `starknet_os_runner` alias,
`starknet-sierra-compile` under `compiler-tools/`, `bootloader_program.json`).
This is what
`scripts/download-deps.sh` downloads. Cut a new one **whenever the pins
change** — `SEQUENCER_TAG`, `PROVING_UTILS_REV`, or `STWO_NIGHTLY`.

1. Update the pins in **both** `build-deps.yml` and `daily-health.yml`
   (and the matching consts in `crates/snip36-cli/src/commands/setup.rs`).
2. Tag and push (incrementing N), or run the workflow manually:
   ```bash
   git tag deps-v5 && git push origin deps-v5
   # or: gh workflow run build-deps.yml -f tag=deps-v5
   ```
3. After it publishes, bump the **`deps-version` file at the repo root** to
   the new tag. That file is the single source of truth for which deps
   release the repo consumes — `scripts/download-deps.sh` defaults to it,
   `daily-health.yml` reads it, and `crates/snip36-cli/build.rs` bakes it
   into `snip36` (`setup --prebuilt` + the deps-mismatch warning).

> The pins and `deps-version` must agree — otherwise CI/local setup fetches
> binaries built from different pins.

## Notes

- Pushing a `v*`/`deps-v*` tag is what triggers a release; nothing publishes on
  a normal merge to `main`.
- The on-chain e2e is **not** part of releasing — it's scheduled / label-gated
  (`run-e2e`); see `AGENTS.md`.
- A failed release run leaves a dangling tag with no (or partial) release.
  Delete the tag (`git push origin :v1.2.0`), fix the cause, and re-tag.
