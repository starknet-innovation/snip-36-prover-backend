# SNIP-36 Virtual OS Stwo Prover

Developer tooling for proving SNIP-36 virtual block execution using the stwo-cairo prover.

## Overview

[SNIP-36](https://community.starknet.io/t/snip-36-virtual-blocks/) introduces **virtual blocks** — off-chain execution of a single `INVOKE_FUNCTION` transaction against a reference Starknet block, proven via the stwo-cairo prover. The virtual OS is a stripped-down Starknet OS (Cairo 1 only, restricted syscalls, single transaction, no block preprocessing).

## Architecture

The project is a **Rust workspace** with a unified CLI (`snip36`) and web backend:

```
┌─────────────────────────────────────────────────────────────────┐
│                  SNIP-36 End-to-End Pipeline                    │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. Deploy & Invoke (snip36 deploy / snip36 fund)               │
│     declare → deploy → invoke → wait for inclusion              │
│                                                                 │
│  2. Prove (snip36 prove virtual-os)                             │
│     ┌──────────────┐   ┌──────────────┐   ┌─────────────────┐  │
│     │ Virtual OS   │──>│ stwo-run-    │──>│ Proof (base64)  │  │
│     │ Execution    │   │ and-prove    │   │ + proof_facts   │  │
│     │ (RPC state)  │   │ (stwo prover)│   │ + L2→L1 msgs    │  │
│     └──────────────┘   └──────────────┘   └────────┬────────┘  │
│                                                     │           │
│  3. Submit (snip36 submit)                          │           │
│     ┌──────────────┐   ┌──────────────┐   ┌────────▼────────┐  │
│     │ Compute tx   │──>│ ECDSA sign   │──>│ RPC             │  │
│     │ hash (with   │   │ (private key)│   │ addInvokeTx     │  │
│     │ proof_facts) │   │              │   │                 │  │
│     └──────────────┘   └──────────────┘   └─────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Prerequisites

- **Rust** — stable (for workspace crates) + `nightly-2025-07-14` (only for `snip36 setup` source builds of the stwo prover)
- **Python 3.12** (not 3.13+) — for the `cairo-compile` venv
- **scarb** — `2.15.2` / Cairo `2.15.0` (emits Sierra 1.7.0 for Sepolia-compatible test contracts)
- **sncast** (Starknet Foundry) — for contract deployment and invocation
- **~10 GB disk** — for cloned repos + built binaries (source builds; prebuilt deps are much smaller)
- **Starknet RPC node** — for state reads during proving

## Supported Platforms

| Platform | Prebuilt deps (`download-deps.sh`) | Docker image |
|----------|------------------------------------|--------------|
| macOS arm64 (Apple Silicon) | ✅ `darwin-arm64` | amd64 only (runs emulated — slow for proving) |
| Linux x86_64 | ✅ `linux-x86_64` | ✅ native `linux/amd64` |
| Linux arm64 (aarch64) | ❌ — build from source via `snip36 setup` | amd64 only (needs QEMU/binfmt) |

> **macOS note:** release assets downloaded with a browser (rather than
> `curl` / `download-deps.sh`) carry the Gatekeeper quarantine attribute.
> Clear it with `xattr -dr com.apple.quarantine <path>`.

## Quick Start

### 1. Build the CLI

```bash
cargo build --release -p snip36-cli
```

Or skip the Rust toolchain entirely and install a prebuilt CLI (available
from the next `v*` release onward):

```bash
curl -fsSL https://github.com/starknet-innovation/snip-36-prover-backend/releases/latest/download/install.sh | sh
```

### 2. Set up external dependencies (prover + runner)

**Fast path — prebuilt binaries (~30 seconds, recommended):**

```bash
snip36 setup --prebuilt
```

Downloads the prebuilt prover stack (stwo prover, virtual-OS runner, sierra
compiler, bootloader) from the pinned `deps-v*` GitHub release for your
platform (see [Supported Platforms](#supported-platforms)), verifies its
checksum, and creates the Python venv for `cairo-compile`.
`./scripts/download-deps.sh [TAG]` does the same without a built CLI.

**From source (~30 minutes — contributors, or platforms without prebuilt assets):**

```bash
snip36 setup
```

This clones the sequencer and proving-utils repos, installs the nightly Rust toolchain, builds the runner and prover binaries, and creates the Python venv for `cairo-compile`.

### 3. Configure environment

```bash
cp .env.example .env
# Edit .env with your account address, private key, RPC URL, and gateway URL
```

Required variables:
- `STARKNET_RPC_URL` — JSON-RPC endpoint (Alchemy, Dwellir, etc., spec v0.8+)
- `STARKNET_ACCOUNT_ADDRESS` — Sender account (hex)
- `STARKNET_PRIVATE_KEY` — Signing key (hex)
- `STARKNET_GATEWAY_URL` — Sequencer gateway for proof submission
  (`https://alpha-sepolia.starknet.io` or `https://alpha-mainnet.starknet.io`).
  Required because RPC nodes don't yet support compressed proofs.

Optional:
- `STARKNET_CHAIN_ID` — `SN_SEPOLIA` (default) or `SN_MAIN`. Must match the
  network of your RPC + gateway — signatures are computed over this chain ID,
  so a mismatch produces `Account: invalid signature`.
- `PROVER_URL` — remote prover JSON-RPC endpoint. If set, `snip36` skips the
  local `starknet_os_runner` and sends `starknet_proveTransaction` to this URL
  instead.

### 4. Check the stack, then run the health check

```bash
snip36 doctor   # offline: validates binaries, bootloader, and venv
snip36 health   # on-chain: RPC, balance, full flow
```

### 5. Run the E2E test

```bash
snip36 e2e                           # counter
snip36 e2e-messages                  # L2→L1 messages
snip36 --env-file .env.mainnet e2e   # run the same flow on mainnet
```

## Networks

The default CI schedule runs against sepolia; mainnet runs are opt-in via
GitHub `workflow_dispatch` (pick `mainnet` from the `network` input). The CI
reads `MAINNET_*` secret equivalents (`MAINNET_STARKNET_RPC_URL`,
`MAINNET_STARKNET_ACCOUNT_ADDRESS`, `MAINNET_STARKNET_PRIVATE_KEY`,
`MAINNET_STARKNET_GATEWAY_URL`) and sets `STARKNET_CHAIN_ID=SN_MAIN` for the
duration of the job.

For a fresh mainnet account, `scripts/deploy_mainnet_account.py` does a
counterfactual OZ-account deploy using the project's pinned class hash
(`sncast account deploy` pins a different OZ class and cannot be used).

## CLI Reference

```bash
snip36 prove virtual-os   # Run virtual OS + stwo prover for a transaction
snip36 prove program       # Prove a compiled Cairo program directly
snip36 prove pie           # Prove a Cairo PIE via bootloader
snip36 submit              # Sign and submit proof via RPC
snip36 deploy account      # Deploy an OZ account contract
snip36 fund                # Transfer STRK from master account
snip36 health              # Run CI health checks
snip36 doctor              # Validate the local proving stack (offline)
snip36 setup               # Install all external dependencies (--prebuilt: download instead of build)
snip36 e2e                 # Full end-to-end test (counter contract)
snip36 e2e-messages        # E2E test for L2→L1 messages (messenger contract)
snip36 e2e-coinflip        # Provable coin flip example (off-chain game)
snip36 e2e-settlement      # E2E settlement test (deposit → prove → settle → payout)
snip36 extract             # Extract virtual OS program
```

Global options: `--env-file <path>`, `--verbose`, `--quiet`

Example application contracts are deployed via the playground backend routes or
with `sncast` directly; the generic CLI only exposes `snip36 deploy account`.

## Web Playground

Interactive web UI for developers to explore the SNIP-36 proving pipeline:

```bash
# Backend (Rust):
cargo run --release -p snip36-playground

# Frontend (React):
cd web/frontend && npm install && npm run dev
```

Open http://localhost:3000

## Docker

Each `v*` release publishes an all-in-one **`snip36` CLI** image (the CLI + the
prebuilt proving stack) to GHCR — no `snip36 setup` needed, proving runs
in-container. The entrypoint is `snip36`, so arguments pass straight through:

```bash
docker run --rm \
  -e STARKNET_RPC_URL=... -e STARKNET_ACCOUNT_ADDRESS=... \
  -e STARKNET_PRIVATE_KEY=... -e STARKNET_GATEWAY_URL=... \
  ghcr.io/starknet-innovation/snip-36-prover-backend:latest \
  prove virtual-os --tx-hash 0x... --block-number N
```

The image bundles the stwo prover, virtual-OS runner, sierra compiler, and
bootloader (linux/amd64). It does **not** include the playground server or
contract-dev tooling (scarb/sncast). See [RELEASING.md](RELEASING.md).

## Full Pipeline (Step by Step)

### Step 1: Prepare an account and deploy/invoke a contract

```bash
snip36 deploy account --public-key $PUBLIC_KEY
snip36 fund --to $TARGET_ADDRESS

# Example app contracts are deployed with sncast (or through the playground backend):
sncast --account myaccount declare --contract-name Counter --url $STARKNET_RPC_URL
sncast --account myaccount deploy --class-hash $CLASS_HASH --url $STARKNET_RPC_URL
sncast --account myaccount invoke --url $STARKNET_RPC_URL \
  --contract-address 0x... --function increment --calldata 0x1
```

### Step 2: Generate the proof

```bash
snip36 prove virtual-os \
  --block-number $((BLOCK_NUMBER - 1)) \
  --tx-hash $TX_HASH \
  --rpc-url $STARKNET_RPC_URL \
  --output output/e2e/e2e.proof
```

### Step 3: Sign and submit

```bash
snip36 submit \
  --proof output/e2e/e2e.proof \
  --proof-facts output/e2e/e2e.proof_facts \
  --calldata "0x1,$CONTRACT_ADDRESS,$FUNCTION_SELECTOR,0x1,0x1" \
  --contract-address $CONTRACT_ADDRESS
```

## Transaction Hash with proof_facts

SNIP-36 extends the standard Starknet v3 invoke transaction hash:

```
Standard:  poseidon(INVOKE, version, sender, tip_rb_hash, paymaster_hash,
                    chain_id, nonce, da_mode, acct_deploy_hash, calldata_hash)

SNIP-36:   poseidon(INVOKE, version, sender, tip_rb_hash, paymaster_hash,
                    chain_id, nonce, da_mode, acct_deploy_hash, calldata_hash,
                    proof_facts_hash)
```

See `crates/snip36-core/src/signing.rs` for the canonical Rust implementation.

## Output Artifacts

After proving, the pipeline generates these files alongside the proof:

| File | Description | When generated |
|------|-------------|----------------|
| `*.proof` | Base64-encoded stwo proof (zstd-compressed) | Always |
| `*.proof_facts` | JSON array of hex field elements (proof identity) | Always |
| `*.raw_messages.json` | L2→L1 messages emitted by the virtual transaction | Only when messages exist |

### L2→L1 Messages (`raw_messages.json`)

When the virtual transaction emits L2→L1 messages (via `send_message_to_l1_syscall`), the prover returns them alongside the proof. These are saved to `raw_messages.json`:

```json
{
  "l2_to_l1_messages": [
    {
      "from_address": "0x153...",
      "payload": ["0x1", "0x2", "0x3"],
      "to_address": "0x123"
    }
  ]
}
```

This is the only channel to transfer data from the virtual transaction to the real verification transaction. The `e2e-messages` test verifies this flow end-to-end using a Messenger contract that calls `send_message_to_l1_syscall`.

## Example: Provable Coin Flip

The `CoinFlip` contract (`tests/contracts/src/lib.cairo`) demonstrates using SNIP-36 virtual blocks as a **verifiable computation oracle** for games:

```
┌─────────────────────────────────────────────────────────────┐
│  Player places bet (0=heads, 1=tails) + public seed         │
│                         │                                    │
│                         ▼                                    │
│  Virtual tx: play(seed, player, bet)                         │
│    outcome = pedersen_hash(seed, player) % 2                 │
│    won = (outcome == bet) ? 1 : 0                            │
│                         │                                    │
│                         ▼                                    │
│  L2→L1 message: [player, seed, bet, outcome, won]            │
│  (settlement receipt — proven by stwo proof)                 │
│                         │                                    │
│                         ▼                                    │
│  L1 contract can trustlessly release payout                  │
└─────────────────────────────────────────────────────────────┘
```

The game logic runs **off-chain** in a virtual block, but the stwo proof guarantees the outcome was honestly computed from the public inputs. Anyone can verify the settlement message without re-executing the game.

```bash
# Play a round (bet=0 for heads, bet=1 for tails)
snip36 e2e-coinflip --env-file .env --bet 0
snip36 e2e-coinflip --env-file .env --bet 1 --prove-only
```

The test deploys the CoinFlip contract, proves a round, and verifies the settlement message matches the expected Poseidon hash computation client-side.

## Project Structure

```
snip-36-prover-backend/
├── Cargo.toml                       # Workspace root
├── crates/                          # SDK — use-case-independent infrastructure
│   ├── snip36-core/                 #   Pure library (config, RPC, signing, proof, types)
│   ├── snip36-cli/                  #   Unified CLI binary (generic + dispatches to apps)
│   └── snip36-server/               #   Server library (generic Axum routes + AppState)
├── apps/                            # Example applications built on the SDK
│   ├── counter/                     #   Counter contract (routes, selectors, e2e, health)
│   ├── messages/                    #   L2→L1 messages (selectors, e2e)
│   ├── coinflip/                    #   CoinFlip game (routes, state, selectors, e2e, settlement)
│   └── playground/                  #   Full server binary (composes SDK + all apps)
├── extractor/                       # Virtual OS program extractor
├── scripts/                         # Shell scripts for external binary orchestration
├── tests/
│   └── contracts/                   # Cairo test contracts (Counter, Messenger, CoinFlip, CoinFlipBank)
├── web/
│   ├── frontend/                    # React + TypeScript playground UI
│   └── coinflip/                    # CoinFlip demo UI
├── sample-input/                    # Prover/bootloader config templates
├── deps/                            # (generated) Cloned repos + built binaries
└── output/                          # (generated) Proofs and artifacts
```

## Key Dependencies

- [starkware-libs/sequencer](https://github.com/starkware-libs/sequencer) @ `PRIVACY-0.14.2-RC.6` — Virtual OS runner (zstd-compressed proofs)
- [starkware-libs/proving-utils](https://github.com/starkware-libs/proving-utils) @ `c0b937bb` — stwo-run-and-prove binary
- [starkware-libs/stwo](https://github.com/starkware-libs/stwo) @ `v2.2.0` — Circle STARK prover
- [starknet-rust-crypto](https://crates.io/crates/starknet-rust-crypto) @ `0.19.1` — Poseidon hash, ECDSA signing

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
