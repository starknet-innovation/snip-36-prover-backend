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

- **Rust** — stable (for workspace crates) + `nightly-2025-07-14` (for stwo prover)
- **sncast** (Starknet Foundry) — for contract deployment and invocation
- **~10 GB disk** — for cloned repos + built binaries
- **Starknet RPC node** — for state reads during proving

## Quick Start

### 1. Build the CLI

```bash
cargo build --release -p snip36-cli
```

### 2. Set up external dependencies (prover + runner)

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
- `STARKNET_RPC_URL` — JSON-RPC endpoint (e.g. Alchemy)
- `STARKNET_ACCOUNT_ADDRESS` — Sender account (hex)
- `STARKNET_PRIVATE_KEY` — Signing key (hex)
- `STARKNET_GATEWAY_URL` — Sequencer gateway for proof submission (e.g. `https://alpha-sepolia.starknet.io`). Required because RPC nodes (pathfinder) don't yet support compressed proofs.

### 4. Run health check

```bash
snip36 health
```

### 5. Run the E2E test

```bash
snip36 e2e
```

## CLI Reference

```bash
snip36 prove virtual-os   # Run virtual OS + stwo prover for a transaction
snip36 prove program       # Prove a compiled Cairo program directly
snip36 prove pie           # Prove a Cairo PIE via bootloader
snip36 submit              # Sign and submit proof via RPC
snip36 deploy account      # Deploy an OZ account contract
snip36 deploy counter      # Declare and deploy a counter contract
snip36 fund                # Transfer STRK from master account
snip36 health              # Run CI health checks
snip36 setup               # Install all external dependencies
snip36 e2e                 # Full end-to-end test (counter contract)
snip36 e2e-messages        # E2E test for L2→L1 messages (messenger contract)
snip36 e2e-coinflip        # Provable coin flip example (off-chain game)
snip36 extract             # Extract virtual OS program
```

Global options: `--env-file <path>`, `--verbose`, `--quiet`

## Web Playground

Interactive web UI for developers to explore the SNIP-36 proving pipeline:

```bash
# Backend (Rust):
cargo run --release -p snip36-playground

# Frontend (React):
cd web/frontend && npm install && npm run dev
```

Open http://localhost:3000

## Full Pipeline (Step by Step)

### Step 1: Deploy and invoke a contract

```bash
snip36 deploy counter
snip36 fund --to $TARGET_ADDRESS
```

Or use `sncast` directly:
```bash
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

- [starkware-libs/sequencer](https://github.com/starkware-libs/sequencer) @ `PRIVACY-0.14.2-RC.2` — Virtual OS runner (zstd-compressed proofs)
- [starkware-libs/proving-utils](https://github.com/starkware-libs/proving-utils) @ `dbc39e7` — stwo-run-and-prove binary
- [starkware-libs/stwo](https://github.com/starkware-libs/stwo) — Circle STARK prover
- [starknet-crypto](https://crates.io/crates/starknet-crypto) — Poseidon hash, ECDSA signing

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
