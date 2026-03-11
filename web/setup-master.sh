#!/usr/bin/env bash
set -euo pipefail

# Set up the master account for the SNIP-36 Playground backend.
#
# The master account is a pre-funded account on Starknet Integration Sepolia
# that funds newly generated dev accounts and deploys contracts on their behalf.
#
# Prerequisites:
#   - sncast (from starknet-foundry)
#   - .env file with STARKNET_ACCOUNT_ADDRESS and STARKNET_PRIVATE_KEY

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Load .env if present
if [ -f "$PROJECT_DIR/.env" ]; then
    set -a
    source "$PROJECT_DIR/.env"
    set +a
fi

STARKNET_RPC_URL="${STARKNET_RPC_URL:?ERROR: STARKNET_RPC_URL is required}"
STARKNET_ACCOUNT_ADDRESS="${STARKNET_ACCOUNT_ADDRESS:?ERROR: STARKNET_ACCOUNT_ADDRESS is required}"
STARKNET_PRIVATE_KEY="${STARKNET_PRIVATE_KEY:?ERROR: STARKNET_PRIVATE_KEY is required}"

echo "Setting up playground master account..."
echo "  RPC:     $STARKNET_RPC_URL"
echo "  Address: $STARKNET_ACCOUNT_ADDRESS"

# Import the master account into sncast as "playground-master"
sncast \
    account import \
    --name playground-master \
    --address "$STARKNET_ACCOUNT_ADDRESS" \
    --private-key "$STARKNET_PRIVATE_KEY" \
    --type oz \
    --url "$STARKNET_RPC_URL" \
    --silent \
    2>&1 || true

# Verify it works by fetching the nonce
NONCE=$(curl -s -X POST "$STARKNET_RPC_URL" \
    -H "Content-Type: application/json" \
    -d "{
        \"jsonrpc\": \"2.0\",
        \"method\": \"starknet_getNonce\",
        \"params\": {
            \"block_id\": \"latest\",
            \"contract_address\": \"$STARKNET_ACCOUNT_ADDRESS\"
        },
        \"id\": 1
    }" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result','ERROR'))")

echo "  Nonce:   $NONCE"

# Pre-compile the counter contract if not already done
CONTRACT_DIR="$PROJECT_DIR/tests/contracts"
if [ -d "$CONTRACT_DIR" ] && command -v scarb &>/dev/null; then
    echo ""
    echo "Compiling counter contract..."
    cd "$CONTRACT_DIR"
    scarb build 2>&1
    echo "  Done."
fi

echo ""
echo "Master account 'playground-master' is ready."
echo ""
echo "Start the backend:"
echo "  cd web/backend && python app.py"
echo ""
echo "Start the frontend:"
echo "  cd web/frontend && npm install && npm run dev"
