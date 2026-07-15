# SNIP-36 E2E Test Suite

End-to-end test that validates the full SNIP-36 virtual block pipeline against the Starknet Sepolia test environment. All tooling is implemented in Rust via the `snip36` CLI.

## Test Flow

```
1. Import funded account into sncast
2. Compile + declare + deploy minimal Cairo counter contract (scarb/sncast)
3. Wait for deploy tx inclusion
4. For each SNOS block:
   a. Construct and sign an invoke transaction (increment)
   b. Prove via virtual OS (`starknet_transaction_prover` + stwo prover)
   c. Sign tx with proof_facts-inclusive hash and submit via the configured
      gateway, falling back to RPC when no gateway is configured
   d. Wait for tx inclusion, verify counter state on-chain
5. Final counter verification
```

## Prerequisites

- `scarb` — contract compilation
- `sncast` — starknet-foundry (declare/deploy/invoke)
- `snip36` CLI built (`cargo build --release -p snip36-cli`)
- `snip36 setup --prebuilt` (or `snip36 setup`) already run, or
  `--prover-url` pointing to a remote prover

## Environment Variables

| Variable | Default | Required |
|----------|---------|----------|
| `STARKNET_RPC_URL` | (see .env) | Yes |
| `STARKNET_ACCOUNT_ADDRESS` | — | Yes |
| `STARKNET_PRIVATE_KEY` | — | Yes |
| `STARKNET_CHAIN_ID` | `SN_SEPOLIA` | No |
| `STARKNET_GATEWAY_URL` | — | No (counter/messages fall back to RPC) |
| `PROVER_URL` | — | No (uses local runner if unset) |

## Running

```bash
target/release/snip36 --env-file .env e2e
```

With options:

```bash
target/release/snip36 --env-file .env e2e \
  --prover-url http://remote:9900 \
  --snos-blocks 3 \
  --counter-increments 5
```

## Files

| File | Description |
|------|-------------|
| `contracts/` | Scarb project containing the E2E contracts |
| `contracts/src/lib.cairo` | Counter, Messenger, CoinFlip, and CoinFlipBank contracts |

The E2E orchestrators live in the app crates, with shared proving, submission,
signing, and RPC logic in the SDK crates:

| Crate | Description |
|-------|-------------|
| `apps/counter/src/e2e.rs` | Counter E2E orchestrator (the generic `snip36 e2e` flow) |
| `apps/messages/src/e2e.rs` | L2→L1 messages E2E orchestrator |
| `apps/coinflip/src/e2e.rs` | CoinFlip E2E orchestrator |
| `apps/coinflip/src/e2e_settlement.rs` | Deposit/prove/settle/payout E2E orchestrator |
| `crates/snip36-cli/src/commands/prove.rs` | Virtual OS proving (`snip36 prove virtual-os`) |
| `crates/snip36-cli/src/commands/submit.rs` | Sign + submit proof via RPC (`snip36 submit`) |
| `crates/snip36-core/src/signing.rs` | Proof_facts-inclusive Poseidon tx hash + signing |
| `crates/snip36-core/src/rpc.rs` | Starknet RPC client (tx polling, calls) |

## CLI Commands

```
snip36 e2e          Full end-to-end test
snip36 prove virtual-os  Run virtual OS + stwo prover
snip36 submit       Sign and submit proof via RPC
snip36 deploy account  Deploy an OpenZeppelin account via sncast
snip36 fund         Transfer STRK from master account
snip36 extract      Extract virtual OS program
snip36 health       CI health check
snip36 setup        Environment setup
```

## Proof Format

The pinned transaction prover + stwo prover outputs proofs in **binary format**
(`ProofFormat::Binary`):

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
target/release/snip36 --env-file .env submit \
    --proof output/e2e/e2e.proof \
    --proof-facts output/e2e/e2e.proof_facts \
    --calldata "0x1,0xCONTRACT,0xSELECTOR,0x1,0x1" \
    --contract-address "0xCONTRACT"
```

## CI

`.github/workflows/daily-health.yml` runs the counter and messages E2E flows on
the daily schedule, on manual dispatch, and on PRs labelled `run-e2e`. Before
provisioning, it verifies that the `deps-version` release metadata matches the
sequencer, proving-utils, and nightly pins and that every required platform
asset exists. It then verifies the generated virtual-OS program hash before
running the messages flow. Unlabelled PRs report the check without running the
on-chain tests.
