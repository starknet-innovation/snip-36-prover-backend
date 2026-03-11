#!/usr/bin/env python3
"""CI health check for the SNIP-36 Proving Playground.

Verifies the integration sepolia environment is still functional:
  1. RPC node reachable
  2. Master account has sufficient STRK balance
  3. OZ Account class hash is declared
  4. Full playground flow: fund → deploy account → deploy counter → invoke → read

Checks 1-3 use pure RPC calls (no external tools).
Check 4 requires sncast + scarb in PATH.

Required env vars:
  STARKNET_RPC_URL            - RPC endpoint
  STARKNET_ACCOUNT_ADDRESS    - Master account address
  STARKNET_PRIVATE_KEY        - Master account private key
"""

import json
import os
import re
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request

# ── Config ──────────────────────────────────────────────

RPC_URL = os.environ.get("STARKNET_RPC_URL", "")
MASTER_ADDRESS = os.environ.get("STARKNET_ACCOUNT_ADDRESS", "")
MASTER_PRIVATE_KEY = os.environ.get("STARKNET_PRIVATE_KEY", "")

# Public contract / class constants
STRK_TOKEN = "0x70a5da4f557b77a9c54546e4bcc900806e28793d8e3eaaa207428d2387249b7"
OZ_ACCOUNT_CLASS_HASH = "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564"

# Starknet selectors (starknet_keccak truncated to 250 bits)
BALANCE_OF_SELECTOR = "0x35a73cd311a05d46deda634c5ee045db92f811b4e74bca4437fcb5302b7af33"
INCREMENT_SELECTOR = "0x7a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d"
GET_COUNTER_SELECTOR = "0x3370263ab53343580e77063a719a5865004caff7f367ec136a6cdd34b6786ca"

MIN_STRK_BALANCE = 1.0  # Minimum STRK balance (in whole tokens)

SNCAST_ACCOUNT = "ci-health-check"
PROJECT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TESTS_DIR = os.path.join(PROJECT_DIR, "tests")

# ── Helpers ─────────────────────────────────────────────

passed = 0
failed = 0


def rpc_call(method: str, params: dict) -> dict:
    """Make a JSON-RPC call to the Starknet node."""
    payload = json.dumps(
        {"jsonrpc": "2.0", "method": method, "params": params, "id": 1}
    ).encode()
    req = urllib.request.Request(
        RPC_URL, data=payload, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def check_pass(name: str, detail: str = ""):
    global passed
    passed += 1
    msg = f"  PASS: {name}"
    if detail:
        msg += f" ({detail})"
    print(msg)


def check_fail(name: str, detail: str = ""):
    global failed
    failed += 1
    msg = f"  FAIL: {name}"
    if detail:
        msg += f" — {detail}"
    print(msg)


def parse_hex(key: str, text: str) -> str | None:
    """Extract a hex value after a key (flexible: underscores ≈ spaces)."""
    pattern = re.compile(key.replace("_", "[_ ]"), re.IGNORECASE)
    for line in text.split("\n"):
        if pattern.search(line):
            m = re.search(r"0x[0-9a-fA-F]+", line)
            if m:
                return m.group()
    return None


def sncast(*args: str, cwd: str | None = None) -> subprocess.CompletedProcess:
    """Run sncast with the CI health check account."""
    cmd = [
        "sncast",
        "--account", SNCAST_ACCOUNT,
        *args,
        "--url", RPC_URL,
    ]
    return subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)


def wait_for_tx(tx_hash: str, timeout: int = 120) -> int | None:
    """Wait for tx confirmation. Returns block number or None."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = rpc_call(
                "starknet_getTransactionReceipt",
                {"transaction_hash": tx_hash},
            )
            if "result" in result:
                receipt = result["result"]
                status = receipt.get("finality_status", "")
                bn = receipt.get("block_number")
                if status in ("ACCEPTED_ON_L2", "ACCEPTED_ON_L1") and bn is not None:
                    return int(bn) if isinstance(bn, int) else int(bn, 16)
        except Exception:
            pass
        time.sleep(3)
    return None


def wait_for_block_after(block_number: int, timeout: int = 120) -> bool:
    """Wait until the chain advances past the given block."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = rpc_call("starknet_blockNumber", {})
            if "result" in result and result["result"] > block_number:
                return True
        except Exception:
            pass
        time.sleep(3)
    return False


# ── Check 1: RPC reachable ──────────────────────────────

def check_rpc():
    print("\n── Check 1: RPC node reachable ──")
    try:
        result = rpc_call("starknet_blockNumber", {})
        block = result.get("result")
        if block and block > 0:
            check_pass("RPC reachable", f"block {block}")
        else:
            check_fail("RPC reachable", f"unexpected response: {result}")
            return

        chain = rpc_call("starknet_chainId", {})
        chain_id = chain.get("result", "")
        check_pass("Chain ID", chain_id)
    except Exception as e:
        check_fail("RPC reachable", str(e))


# ── Check 2: Master account balance ────────────────────

def check_balance():
    print("\n── Check 2: Master account STRK balance ──")
    try:
        result = rpc_call(
            "starknet_call",
            {
                "request": {
                    "contract_address": STRK_TOKEN,
                    "entry_point_selector": BALANCE_OF_SELECTOR,
                    "calldata": [MASTER_ADDRESS],
                },
                "block_id": "latest",
            },
        )
        if "error" in result:
            check_fail("Balance check", json.dumps(result["error"]))
            return

        values = result.get("result", [])
        if not values:
            check_fail("Balance check", "empty result")
            return

        # u256 = low + high * 2^128
        low = int(values[0], 16)
        high = int(values[1], 16) if len(values) > 1 else 0
        balance_wei = low + (high << 128)
        balance_strk = balance_wei / 10**18

        if balance_strk >= MIN_STRK_BALANCE:
            check_pass("STRK balance", f"{balance_strk:.2f} STRK")
        else:
            check_fail(
                "STRK balance too low",
                f"{balance_strk:.2f} STRK < {MIN_STRK_BALANCE} STRK minimum",
            )
    except Exception as e:
        check_fail("Balance check", str(e))


# ── Check 3: OZ class declared ─────────────────────────

def check_oz_class():
    print("\n── Check 3: OZ Account class declared ──")
    try:
        result = rpc_call(
            "starknet_getClass",
            {"block_id": "latest", "class_hash": OZ_ACCOUNT_CLASS_HASH},
        )
        if "error" in result:
            check_fail("OZ class declared", json.dumps(result["error"]))
        elif "result" in result:
            check_pass("OZ class declared", OZ_ACCOUNT_CLASS_HASH[:18] + "...")
        else:
            check_fail("OZ class declared", "unexpected response")
    except Exception as e:
        check_fail("OZ class declared", str(e))


# ── Check 4: Full playground flow ───────────────────────

def check_full_flow():
    print("\n── Check 4: Full playground flow ──")

    # Check prerequisites
    for cmd in ("sncast", "scarb"):
        if not any(
            os.access(os.path.join(p, cmd), os.X_OK)
            for p in os.environ.get("PATH", "").split(":")
        ):
            check_fail(f"{cmd} not in PATH", "skipping full flow")
            return

    # Import account
    subprocess.run(
        [
            "sncast", "account", "import",
            "--name", SNCAST_ACCOUNT,
            "--address", MASTER_ADDRESS,
            "--private-key", MASTER_PRIVATE_KEY,
            "--type", "oz",
            "--url", RPC_URL,
            "--silent",
        ],
        capture_output=True,
        text=True,
    )

    # 4a: Compile + declare counter contract
    print("  Compiling counter contract...")
    contracts_dir = os.path.join(TESTS_DIR, "contracts")
    build = subprocess.run(
        ["scarb", "build"], capture_output=True, text=True, cwd=contracts_dir
    )
    if build.returncode != 0:
        check_fail("Compile counter", build.stderr)
        return
    check_pass("Compile counter")

    print("  Declaring counter class...")
    declare = sncast("declare", "--contract-name", "Counter", cwd=contracts_dir)
    output = declare.stdout + declare.stderr
    class_hash = parse_hex("class_hash", output)
    if not class_hash:
        class_hash = re.search(r"0x[0-9a-fA-F]{50,}", output)
        class_hash = class_hash.group() if class_hash else None
    if class_hash:
        check_pass("Declare counter", class_hash[:18] + "...")
    else:
        check_fail("Declare counter", output[:200])
        return

    # 4b: Deploy counter
    print("  Deploying counter contract...")
    salt = "0x" + secrets.token_hex(16)
    deploy = sncast("deploy", "--class-hash", class_hash, "--salt", salt)
    contract_address = parse_hex("contract_address", deploy.stdout)
    deploy_tx = parse_hex("transaction_hash", deploy.stdout)
    if not contract_address:
        check_fail("Deploy counter", (deploy.stderr or deploy.stdout)[:200])
        return
    check_pass("Deploy counter", contract_address[:18] + "...")

    if deploy_tx:
        bn = wait_for_tx(deploy_tx)
        if bn:
            wait_for_block_after(bn)
            check_pass("Deploy tx confirmed", f"block {bn}")
        else:
            check_fail("Deploy tx confirmation", "timeout")
            return

    # 4c: Invoke increment
    print("  Invoking increment(1)...")
    invoke = sncast(
        "invoke",
        "--contract-address", contract_address,
        "--function", "increment",
        "--calldata", "0x1",
    )
    invoke_tx = parse_hex("transaction_hash", invoke.stdout)
    if not invoke_tx:
        check_fail("Invoke increment", (invoke.stderr or invoke.stdout)[:200])
        return
    check_pass("Invoke increment", invoke_tx[:18] + "...")

    bn = wait_for_tx(invoke_tx)
    if bn:
        check_pass("Invoke tx confirmed", f"block {bn}")
    else:
        check_fail("Invoke tx confirmation", "timeout")
        return

    # 4d: Read counter
    print("  Reading counter value...")
    try:
        result = rpc_call(
            "starknet_call",
            {
                "request": {
                    "contract_address": contract_address,
                    "entry_point_selector": GET_COUNTER_SELECTOR,
                    "calldata": [],
                },
                "block_id": "latest",
            },
        )
        values = result.get("result", [])
        counter = int(values[0], 16) if values else 0
        if counter >= 1:
            check_pass("Counter value", str(counter))
        else:
            check_fail("Counter value", f"expected >= 1, got {counter}")
    except Exception as e:
        check_fail("Read counter", str(e))


# ── Main ────────────────────────────────────────────────

def main():
    if not RPC_URL:
        print("ERROR: STARKNET_RPC_URL is required")
        sys.exit(1)
    if not MASTER_ADDRESS:
        print("ERROR: STARKNET_ACCOUNT_ADDRESS is required")
        sys.exit(1)

    print("=== SNIP-36 Playground Health Check ===")
    print(f"  RPC:     {RPC_URL}")
    print(f"  Account: {MASTER_ADDRESS[:10]}...{MASTER_ADDRESS[-6:]}")

    check_rpc()
    check_balance()
    check_oz_class()

    if MASTER_PRIVATE_KEY:
        check_full_flow()
    else:
        print("\n── Check 4: Skipped (no STARKNET_PRIVATE_KEY) ──")

    print(f"\n{'=' * 42}")
    print(f"  Passed: {passed}")
    print(f"  Failed: {failed}")
    print(f"{'=' * 42}")

    if failed > 0:
        print(f"\n  RESULT: {failed} CHECK(S) FAILED")
        sys.exit(1)
    else:
        print("\n  RESULT: ALL CHECKS PASSED")
        sys.exit(0)


if __name__ == "__main__":
    main()
