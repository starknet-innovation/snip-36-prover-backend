#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DEPS_DIR="$PROJECT_DIR/deps"
OUTPUT_DIR="$PROJECT_DIR/output"
VENV_DIR="$PROJECT_DIR/sequencer_venv"
STWO_NIGHTLY="nightly-2025-07-14"

usage() {
    echo "Usage: $0 --block-number <N> --tx-hash <HASH> --rpc-url <URL> [OPTIONS]"
    echo ""
    echo "Run the virtual OS to produce a proof for a transaction."
    echo ""
    echo "This script sends a starknet_proveTransaction request to either a"
    echo "remote prover (--prover-url) or a locally started starknet_os_runner."
    echo ""
    echo "Required:"
    echo "  --block-number <N>   Reference Starknet block number"
    echo "  --tx-hash <HASH>     Transaction hash to prove"
    echo "  --rpc-url <URL>      Starknet RPC endpoint URL"
    echo ""
    echo "Options:"
    echo "  --prover-url <URL>       Use a remote prover (skip local server startup)"
    echo "  --output <path>          Output proof path (default: output/virtual_os.proof)"
    echo "  --port <port>            Port for local runner server (default: 9900)"
    echo "  --strk-fee-token <ADDR>  Override STRK fee token address (for custom networks)"
    echo "  -h, --help               Show this help"
    exit 0
}

BLOCK_NUMBER=""
TX_HASH=""
RPC_URL=""
PROVER_URL=""
PROOF_OUTPUT="$OUTPUT_DIR/virtual_os.proof"
RUNNER_PORT=9900
STRK_FEE_TOKEN=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --block-number)
            BLOCK_NUMBER="$2"
            shift 2
            ;;
        --tx-hash)
            TX_HASH="$2"
            shift 2
            ;;
        --rpc-url)
            RPC_URL="$2"
            shift 2
            ;;
        --prover-url)
            PROVER_URL="$2"
            shift 2
            ;;
        --output)
            PROOF_OUTPUT="$2"
            shift 2
            ;;
        --port)
            RUNNER_PORT="$2"
            shift 2
            ;;
        --strk-fee-token)
            STRK_FEE_TOKEN="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

if [ -z "$BLOCK_NUMBER" ] || [ -z "$TX_HASH" ] || [ -z "$RPC_URL" ]; then
    echo "ERROR: --block-number, --tx-hash, and --rpc-url are all required."
    echo ""
    usage
fi

mkdir -p "$OUTPUT_DIR"

echo "=== Running Virtual OS (Phase 1) ==="
echo "  Block:   $BLOCK_NUMBER"
echo "  Tx:      $TX_HASH"
echo "  RPC:     $RPC_URL"
echo "  Output:  $PROOF_OUTPUT"

# Fetch the transaction from the RPC to get its full data
echo "Fetching transaction $TX_HASH from RPC..."
TX_RESPONSE=$(curl -s -X POST "$RPC_URL" \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\",
        \"method\": \"starknet_getTransactionByHash\",
        \"params\": {\"transaction_hash\": \"$TX_HASH\"},
        \"id\": 1
    }")

TX_DATA=$(echo "$TX_RESPONSE" | jq '.result')
if [ "$TX_DATA" = "null" ] || [ -z "$TX_DATA" ]; then
    echo "ERROR: Could not fetch transaction $TX_HASH"
    echo "  Response: $TX_RESPONSE"
    exit 1
fi
echo "  Transaction fetched successfully"

# Determine the prover endpoint
if [ -n "$PROVER_URL" ]; then
    # Use remote prover directly
    PROVE_ENDPOINT="$PROVER_URL"
    echo "  Prover:  $PROVE_ENDPOINT (remote)"
else
    # Start local starknet_os_runner
    if [ ! -d "$DEPS_DIR/sequencer" ]; then
        echo "ERROR: deps/sequencer/ not found and no --prover-url specified."
        echo "Either run ./scripts/setup.sh or provide --prover-url."
        exit 1
    fi

    RUNNER_BIN="$DEPS_DIR/sequencer/target/release/starknet_os_runner"
    if [ ! -f "$RUNNER_BIN" ]; then
        echo "Building starknet_os_runner (toolchain: $STWO_NIGHTLY, feature: stwo_proving)..."
        (
            if [ -d "$VENV_DIR" ]; then
                export PATH="$VENV_DIR/bin:$PATH"
            fi
            cargo +"$STWO_NIGHTLY" build --release \
                --manifest-path "$DEPS_DIR/sequencer/Cargo.toml" \
                -p starknet_os_runner --features stwo_proving
        )
    fi

    if [ ! -f "$RUNNER_BIN" ]; then
        echo "ERROR: starknet_os_runner binary not found after build."
        exit 1
    fi

    if [ -z "${STWO_RUN_AND_PROVE_PATH:-}" ]; then
        export STWO_RUN_AND_PROVE_PATH="$DEPS_DIR/bin/stwo-run-and-prove"
    fi

    RUNNER_EXTRA_ARGS=()
    if [ -n "$STRK_FEE_TOKEN" ]; then
        RUNNER_EXTRA_ARGS+=(--strk-fee-token-address "$STRK_FEE_TOKEN")
        echo "  STRK token: $STRK_FEE_TOKEN"
    fi

    echo "Starting starknet_os_runner on port $RUNNER_PORT..."
    "$RUNNER_BIN" \
        --rpc-url "$RPC_URL" \
        --chain-id SN_SEPOLIA \
        --port "$RUNNER_PORT" \
        --ip 127.0.0.1 \
        --prefetch-state false \
        ${RUNNER_EXTRA_ARGS[@]+"${RUNNER_EXTRA_ARGS[@]}"} &
    RUNNER_PID=$!
    cleanup() {
        local rc=$?
        kill $RUNNER_PID 2>/dev/null
        wait $RUNNER_PID 2>/dev/null || true
        return $rc
    }
    trap cleanup EXIT

    for i in $(seq 1 30); do
        if curl -s "http://127.0.0.1:$RUNNER_PORT/" >/dev/null 2>&1; then
            break
        fi
        if ! kill -0 "$RUNNER_PID" 2>/dev/null; then
            echo "ERROR: starknet_os_runner exited prematurely"
            wait "$RUNNER_PID" 2>/dev/null || true
            exit 1
        fi
        sleep 1
    done
    echo "  Server ready"

    PROVE_ENDPOINT="http://127.0.0.1:$RUNNER_PORT"
fi

echo ""

# Call starknet_proveTransaction via JSON-RPC
echo "Sending starknet_proveTransaction request..."
PROVE_RESPONSE=$(curl -s -X POST "$PROVE_ENDPOINT" \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\",
        \"method\": \"starknet_proveTransaction\",
        \"params\": {
            \"block_id\": {\"block_number\": $BLOCK_NUMBER},
            \"transaction\": $TX_DATA
        },
        \"id\": 1
    }" \
    --max-time 600)

# Check for JSON-RPC errors
RPC_ERROR=$(echo "$PROVE_RESPONSE" | jq -r '.error // empty')
if [ -n "$RPC_ERROR" ] && [ "$RPC_ERROR" != "" ] && [ "$RPC_ERROR" != "null" ]; then
    echo "ERROR: starknet_proveTransaction failed"
    echo "  $RPC_ERROR"
    exit 1
fi

RESULT=$(echo "$PROVE_RESPONSE" | jq '.result')
if [ "$RESULT" = "null" ] || [ -z "$RESULT" ]; then
    echo "ERROR: Empty result from starknet_proveTransaction"
    echo "  Response: $PROVE_RESPONSE"
    exit 1
fi

# Save proof
mkdir -p "$(dirname "$PROOF_OUTPUT")"
echo "$RESULT" | jq -r '.proof' > "$PROOF_OUTPUT"

# Also save proof_facts alongside
FACTS_OUTPUT="${PROOF_OUTPUT%.proof}.proof_facts"
echo "$RESULT" | jq '.proof_facts' > "$FACTS_OUTPUT"

echo ""
echo "=== Virtual OS execution complete ==="
echo "  Proof:       $PROOF_OUTPUT"
echo "  Proof facts: $FACTS_OUTPUT"
if [ -f "$PROOF_OUTPUT" ]; then
    PROOF_SIZE=$(wc -c < "$PROOF_OUTPUT" | tr -d ' ')
    echo "  Proof size:  $PROOF_SIZE bytes"
fi
