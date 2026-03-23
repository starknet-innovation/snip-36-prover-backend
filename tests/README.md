# SNIP-36 E2E Test Suite

End-to-end test that validates the full SNIP-36 virtual block pipeline against the Starknet Sepolia test environment. All tooling is implemented in Rust via the `snip36` CLI.

## Test Flow

```
1. Import funded account into sncast
2. Compile + declare + deploy minimal Cairo counter contract (scarb/sncast)
3. Wait for deploy tx inclusion
4. For each SNOS block:
   a. Construct and sign an invoke transaction (increment)
   b. Prove via virtual OS (starknet_os_runner + stwo prover)
   c. Sign tx with proof_facts-inclusive hash and submit via RPC
   d. Wait for tx inclusion, verify counter state on-chain
5. Final counter verification
```

## Prerequisites

- `scarb` — contract compilation
- `sncast` — starknet-foundry (declare/deploy/invoke)
- `snip36` CLI built (`cargo build --release -p snip36-cli`)
- `snip36 setup` already run (prover + runner built), or `--prover-url` pointing to a remote prover

## Environment Variables

| Variable | Default | Required |
|----------|---------|----------|
| `STARKNET_RPC_URL` | (see .env) | Yes |
| `STARKNET_ACCOUNT_ADDRESS` | — | Yes |
| `STARKNET_PRIVATE_KEY` | — | Yes |
| `STARKNET_CHAIN_ID` | `SN_SEPOLIA` | No |
| `PROVER_URL` | — | No (uses local runner if unset) |

## Running

```bash
source .env
./snip36 e2e
```

With options:

```bash
./snip36 e2e --prover-url http://remote:9900 --snos-blocks 3 --counter-increments 5
```

## Files

| File | Description |
|------|-------------|
| `contracts/` | Minimal Cairo counter contract (Scarb project) |
| `contracts/src/lib.cairo` | Counter contract: `increment(amount)` + `get_counter()` |

The E2E orchestrator and all supporting logic (tx signing, proof submission, tx polling) live in the `snip36` CLI crate:

| Crate | Description |
|-------|-------------|
| `crates/snip36-cli/src/commands/e2e.rs` | E2E test orchestrator |
| `crates/snip36-cli/src/commands/prove.rs` | Virtual OS proving (`snip36 prove virtual-os`) |
| `crates/snip36-cli/src/commands/submit.rs` | Sign + submit proof via RPC (`snip36 submit`) |
| `crates/snip36-core/src/signing.rs` | Proof_facts-inclusive Poseidon tx hash + signing |
| `crates/snip36-core/src/rpc.rs` | Starknet RPC client (tx polling, calls) |

## CLI Commands

```
snip36 e2e          Full end-to-end test
snip36 prove        Run virtual OS + stwo prover
snip36 submit       Sign and submit proof via RPC
snip36 deploy       Deploy contracts via sncast
snip36 fund         Transfer STRK from master account
snip36 extract      Extract virtual OS program
snip36 health       CI health check
snip36 setup        Environment setup
```

## Proof Format

The DEMO-19 runner + stwo prover outputs proofs in **binary format** (`ProofFormat::Binary`):

1. Prover: `CairoProofForRustVerifier` → `bincode::serialize` → bzip2 → file
2. Runner: decompresses → encodes to `Vec<u32>` (BE + padding prefix) → base64 string
3. The proof is returned as a base64 string in the JSON-RPC response

The `proof_facts` are a JSON array of hex felt values containing:
- `PROOF0` marker
- `VIRTUAL_SNOS` marker
- Virtual OS program hash
- `VIRTUAL_SNOS0` marker
- Block number, block hash, OS config hash
- L2→L1 message count and hashes

## Transaction Signing

Proof-bearing transactions require the `proof_facts` to be included in the Poseidon transaction hash chain. Standard Starknet SDKs (starknet-py, starknet.js) do **not** include this, producing an incorrect hash and "invalid signature" errors.

The `snip36` CLI handles this natively via `snip36_core::signing`, which computes the correct hash:

```bash
./snip36 submit \
    --proof output/e2e/e2e.proof \
    --proof-facts output/e2e/e2e.proof_facts \
    --calldata "0x1,0xCONTRACT,0xSELECTOR,0x1,0x1" \
    --contract-address "0xCONTRACT"
```

## CI

A daily health check runs via GitHub Actions (`.github/workflows/daily-health.yml`), executing `snip36 health` to verify the sepolia environment is operational.
