# Sample Input Documentation

## Prover Parameters (`prover_params.json`)

Configuration for the stwo prover. These are suitable for development/testing (low security, fast proving):

| Field | Description |
|-------|-------------|
| `channel_hash` | Hash function for Fiat-Shamir channel (`blake2s`) |
| `pcs_config.pow_bits` | Proof-of-work difficulty bits (0 = disabled, 26 for production) |
| `pcs_config.fri_config.log_blowup_factor` | FRI blowup factor as log2 (1 = 2x blowup) |
| `pcs_config.fri_config.log_last_layer_degree_bound` | Degree bound for the last FRI layer |
| `pcs_config.fri_config.n_queries` | Number of FRI queries (more = more secure but larger proof) |
| `pcs_config.fri_config.fold_step` | Folding step for FRI |
| `preprocessed_trace` | Preprocessed trace type (`canonical_without_pedersen`) |
| `include_all_preprocessed_columns` | Include all preprocessed columns in proof |
| `store_polynomials_coefficients` | Store polynomial coefficients |
| `channel_salt` | Salt for channel hash |

For production, increase `n_queries` to 70 and `pow_bits` to 26.

**Note:** These parameters are used when invoking `stwo-run-and-prove` directly (see below). The `starknet_os_runner` (used in the E2E pipeline) has its own built-in prover parameters.

## Bootloader Input Template (`bootloader_input_template.json`)

Template for `SimpleBootloaderInput`, used when proving a compiled Cairo program through the bootloader. Replace `{{PROGRAM_PATH}}` with the **absolute** path to a compiled Cairo 0 program JSON (`cairo-compile`, non-proof-mode).

### SimpleBootloaderInput Format

```json
{
  "tasks": [
    {
      "path": "<absolute-path-to-compiled-program.json>",
      "program_hash_function": "poseidon",
      "type": "RunProgramTask"
    }
  ],
  "single_page": true
}
```

Field notes:

- `tasks[].path` — absolute path to the compiled program the bootloader should run.
- `tasks[].program_hash_function` — `poseidon` or `pedersen`. `pedersen` requires
  `preprocessed_trace: "canonical"` in the prover params (the default
  `canonical_without_pedersen` here panics with
  `Missing pedersen points in the preprocessed trace`).
- `single_page` — required; controls output page packing.

The bootloader runs the task program, hashes it, and produces a proof of correct execution.

### Direct proving example

All `stwo-run-and-prove` paths must be absolute:

```bash
stwo-run-and-prove \
  --program /abs/path/deps/bin/bootloader_program.json \
  --program_input /abs/path/bootloader_input.json \
  --prover_params_json /abs/path/sample-input/prover_params.json \
  --proof_path /abs/path/out.proof \
  --proof-format binary --verify
```

## Proof Output Formats

The stwo prover supports multiple output formats:

| Format | Description | Used by |
|--------|-------------|---------|
| `binary` | `bincode(CairoProofForRustVerifier)` + bzip2 compression | `starknet_os_runner` (default) |
| `cairo-serde` | JSON array of hex field elements | direct `stwo-run-and-prove` runs |
| `json` | Full proof structure as JSON | debugging |

The E2E pipeline uses **binary** format. The runner decompresses the bzip2 file, encodes the bincode bytes as big-endian packed `u32` values, and returns the result as a base64 string.

## Proof Facts Format

The virtual OS produces `proof_facts` — a JSON array of hex felt strings that identify the proven execution:

```json
[
  "0x50524f4f4630",          // PROOF0 marker
  "0x5649525455414c5f534e4f53", // VIRTUAL_SNOS marker
  "0x974341...",              // Virtual OS program hash
  "0x5649525455414c5f534e4f5330", // VIRTUAL_SNOS0 marker
  "0x186a64",                // Block number (hex)
  "0x7da482...",              // Block hash
  "0x6989a6...",              // OS config hash
  "0x0"                      // L2→L1 message count (0 = no messages)
]
```

These are included in the transaction hash computation (Poseidon hash chain) and submitted alongside the proof via RPC.

## L2→L1 Messages (`raw_messages.json`)

When the virtual transaction emits L2→L1 messages (via `send_message_to_l1_syscall`), the prover returns them in the `l2_to_l1_messages` field. The CLI saves these to a `*.raw_messages.json` file alongside the proof:

```json
{
  "l2_to_l1_messages": [
    {
      "from_address": "0x6ff654...",
      "payload": ["0x1", "0x2", "0x3"],
      "to_address": "0x123"
    }
  ]
}
```

| Field | Description |
|-------|-------------|
| `from_address` | Contract address that emitted the message |
| `to_address` | L1 destination address |
| `payload` | Array of hex felt strings |

This file is only generated when at least one L2→L1 message is present. It is the only mechanism to transfer data from the virtual transaction to the real verification transaction.
