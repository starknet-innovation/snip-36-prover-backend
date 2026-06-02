# AGENTS.md

## Project Overview

SNIP-36 virtual block proving tooling for Starknet. Two-phase pipeline:
1. **Execute** — Run virtual OS against RPC node → produces Cairo PIE
2. **Prove** — Feed PIE through bootloader into stwo prover → produces stwo proof

## Architecture

Rust workspace split into reusable SDK crates and example apps built on top of them.

**SDK (`crates/`) — use-case-independent infrastructure:**
- `crates/snip36-core/` — Pure library: typed config, Starknet RPC client, SNIP-36 signing, proof encoding/types
- `crates/snip36-cli/` — Unified CLI (`snip36`); owns generic subcommands (prove, submit, deploy, fund, setup, extract) and dispatches health/e2e to the apps
- `crates/snip36-server/` — Server library: generic Axum routes + `AppState` (composed by app server binaries)

**Apps (`apps/`) — example applications built on the SDK:**
- `apps/counter/` — Counter contract demo **and the reference app**: also backs the generic `snip36 health` and `snip36 e2e` commands (routes, selectors, e2e, health). The name "counter" doesn't imply those are counter-specific.
- `apps/messages/` — L2→L1 messages (selectors, e2e)
- `apps/coinflip/` — CoinFlip game (routes, state, selectors, e2e, settlement)
- `apps/playground/` — Full server binary (`snip36-playground`) composing the SDK + all apps

**Other:**
- `extractor/` — Rust crate that extracts the compiled virtual OS program (excluded from default workspace, requires `deps/sequencer/`)
- `web/frontend/` — React + TypeScript playground UI
- `web/coinflip/` — CoinFlip demo UI
- `tests/contracts/` — Cairo test contracts (Counter, Messenger, CoinFlip, CoinFlipBank) for E2E tests
- `scripts/` — Shell scripts for external binary orchestration (setup, prove, run-virtual-os)
- `sample-input/` — Template inputs for the prover and bootloader
- `deps/` — (generated, gitignored) Cloned repos: `proving-utils`, `sequencer`

## Key Conventions

- All Rust code targets stable toolchain (workspace crates)
- External dependencies (`stwo`, `sequencer`) require `nightly-2025-07-14`
- Config loaded from `.env` via `snip36_core::Config`
- Structured logging via `tracing` crate
- Error handling via `color-eyre` (CLI) and typed errors (`thiserror`) in core
- All proof output is base64-encoded (runner outputs directly as base64)
- Proofs and build artifacts go in `output/` (gitignored)

## Building

```bash
cargo build --workspace                 # Build all crates + apps
cargo build --release -p snip36-cli     # Build the CLI
cargo build --release -p snip36-playground  # Build the web backend binary

# External dependencies (stwo prover, starknet_os_runner):
snip36 setup                         # Install external deps
```

## CLI Usage

```bash
snip36 prove virtual-os --block-number N --tx-hash 0x... --rpc-url URL
snip36 prove program --program file.json --output proof.out
snip36 submit --proof proof.b64 --proof-facts facts.json --calldata 0x1,0x2 --contract-address 0x...
snip36 deploy account --public-key 0x...
snip36 fund --to 0x... --amount 10000000000000000000
snip36 health
snip36 health --quick
snip36 setup
snip36 e2e
snip36 e2e-messages          # E2E test for L2→L1 messages (messenger contract)
snip36 e2e-coinflip          # Provable coin flip example (off-chain game)
snip36 e2e-settlement        # E2E settlement: deposit → prove → settle → payout
```

## Web Playground

```bash
# Backend (Rust):
cargo run --release -p snip36-playground

# Frontend (React):
cd web/frontend && npm install && npm run dev
```

## Verifying a change

Work down this ladder — most changes can be validated entirely at the cheap,
offline tiers. Prefer adding an offline unit test over relying on the on-chain
e2e.

**Tier 1 — fast, offline, no secrets (run these first; this is what CI gates on):**

```bash
cargo build --workspace --all-features    # --all-features builds the `cli`-gated code
cargo test  --workspace --all-features    # unit tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Tier 1 covers the pure logic: SNIP-36 tx-hash + signing (`snip36-core::signing`),
proof encoding / `proof_facts` parsing (`snip36-core::proof`), resource-bound
JSON (`snip36-core::types`), sncast-output parsing (`snip36-core::cli_util`,
behind the `cli` feature → needs `--all-features`), and the coin-flip outcome
math (`apps/coinflip/src/outcome.rs`). If you change any of these, add/extend a
test here.

**Tier 2 — on-chain e2e (needs secrets + external deps; NOT runnable in a sandbox):**

```bash
snip36 health          # Sepolia health check (needs RPC)
snip36 e2e             # full flow: execute → prove → sign → submit
snip36 e2e-messages    # L2→L1 messages: deploy Messenger → prove → verify raw_messages.json
snip36 e2e-coinflip    # provable coin flip: deploy CoinFlip → prove → verify settlement message
snip36 e2e-settlement  # full settlement: deposit → prove → settle → payout
```

Tier 2 requires `snip36 setup` (or `./scripts/download-deps.sh deps-v3` for the
prebuilt binaries), a funded account, and RPC + gateway in `.env`. In CI it is
**not** a per-PR gate — it runs on the daily schedule, on `workflow_dispatch`,
and on PRs labelled `run-e2e` (see `.github/workflows/daily-health.yml`).
Changes to the proving/submission path ultimately need Tier 2, but extract the
verifiable part into a pure function and unit-test it at Tier 1 where you can.

## Environment

- `.env` contains secrets (RPC URL, private key) — never commit
- `.env.example` shows required variables
- Target network: Starknet Sepolia

## Working with Proofs

- PIE files: `.pie.zip` — Cairo Program Independent Execution artifacts
- Proof files: `.proof` — stwo proofs as base64 strings
- Proof facts: `.proof_facts` — JSON array of hex felt strings identifying the proven execution
- L2→L1 messages: `.raw_messages.json` — saved when the virtual tx emits messages (only data channel from virtual tx to real tx)
- The `proof_facts` field in INVOKE_TXN_V3 must be included in Poseidon tx hash computation (non-standard — see `crates/snip36-core/src/signing.rs`)

## Common Pitfalls

- Runner must use `--prefetch-state false` (prefetch has a bug with missing storage keys)
- Tx signing must include `proof_facts` in the hash chain — standard Starknet SDKs do NOT do this
- L2 gas for proof verification is ~75M — set max to ≥117M
- The `extractor` crate requires `deps/sequencer/` to exist (run `snip36 setup` first)
