# SNIP-36 Proving Playground

Interactive web UI for developers to explore the SNIP-36 proving pipeline using a simple counter contract.

## What it does

Walks through the full SNIP-36 flow step by step:

1. **Generate Stark key pair** in-browser (no wallet needed)
2. **Fund** the generated account from a master account
3. **Deploy** an OpenZeppelin account contract
4. **Deploy** a Counter contract
5. **Invoke** `increment(1)` — tx hash computed and signed entirely in-browser
6. **Prove** the transaction using virtual OS + stwo prover (streamed logs)
7. **Submit** a proof-bearing transaction via RPC

Each step includes an explainer toggle with educational context.

## Quick Start

```bash
# 1. Configure environment (one-time)
cp .env.example .env
# Edit .env with your funded account credentials:
#   STARKNET_RPC_URL, STARKNET_ACCOUNT_ADDRESS, STARKNET_PRIVATE_KEY

# 2. Start the backend (Rust) — imports sncast account automatically on startup
cargo run --release -p snip36-server

# 3. Start the frontend (in another terminal)
cd web/frontend
npm install
npm run dev
```

Open http://localhost:3000

## Architecture

- **Frontend** (React + starknet.js v9): Key generation, tx hash computation, and signing happen entirely in-browser. The private key never leaves the client.
- **Backend** (Axum / Rust): REST + SSE API backed by `snip36-core` for signing and the shell scripts for prover orchestration. Holds a pre-funded master account for funding dev accounts.
- **Proof streaming**: SSE (Server-Sent Events) streams prover logs in real-time.

### Transaction Hashing

Two different hash computations are used:

| Step | Hash | Resource Bounds |
|------|------|-----------------|
| Step 5 (normal invoke via RPC) | Standard SNIP-8 | L1_GAS + L2_GAS |
| Step 7 (proof tx via RPC) | SNIP-36 extended | L1_GAS + L2_GAS + L1_DATA + proof_facts |

Step 5 uses starknet.js's built-in `calculateInvokeTransactionHash`. Step 7 uses `snip36-core::signing` in the Rust backend.

## Prerequisites

- Rust stable toolchain
- Node.js 18+
- `sncast` (from starknet-foundry)
- The prover tooling set up via `snip36 setup`
- A funded account on Starknet Sepolia (configure in `.env`)
