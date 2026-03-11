"""SNIP-36 Proving Playground — Backend API.

Wraps existing shell scripts and Python tooling behind a REST + SSE API.
A pre-funded master account handles funding newly generated dev accounts.
"""

import asyncio
import json
import logging
import os
import re
import subprocess
import sys
from pathlib import Path

logging.basicConfig(level=logging.INFO)
log = logging.getLogger("playground")

from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
from sse_starlette.sse import EventSourceResponse
from master_account import MasterAccount

PROJECT_DIR = Path(__file__).resolve().parent.parent.parent
WEB_DIR = PROJECT_DIR / "web"

# Load web/.env if present
_env_file = WEB_DIR / ".env"
if _env_file.exists():
    for line in _env_file.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            key, _, value = line.partition("=")
            os.environ.setdefault(key.strip(), value.strip())
SCRIPTS_DIR = PROJECT_DIR / "scripts"
TESTS_DIR = PROJECT_DIR / "tests"
OUTPUT_DIR = PROJECT_DIR / "output" / "playground"

app = FastAPI(title="SNIP-36 Proving Playground")
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["*"],
    allow_headers=["*"],
)

# In-memory session state (single-user demo)
sessions: dict[str, dict] = {}

# Master account for funding
master = MasterAccount(
    rpc_url=os.environ.get("STARKNET_RPC_URL", "http://34.170.239.64:9545/rpc/v0_10"),
    address=os.environ.get(
        "MASTER_ACCOUNT_ADDRESS",
        os.environ.get("STARKNET_ACCOUNT_ADDRESS", ""),
    ),
    private_key=os.environ.get(
        "MASTER_PRIVATE_KEY",
        os.environ.get("STARKNET_PRIVATE_KEY", ""),
    ),
)


# ── Selectors (computed at startup) ──────────────────────

INCREMENT_SELECTOR = "0x7a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d"
GET_COUNTER_SELECTOR = "0x3370263ab53343580e77063a719a5865004caff7f367ec136a6cdd34b6786ca"

# ── Resource bounds ───────────────────────────────────────
# l1_gas + l2_gas are included in the tx hash (per SNIP-8).
# l1_data_gas is in the RPC payload but NOT in the hash.

RESOURCE_BOUNDS_FOR_RPC = {
    "l1_gas": {"max_amount": "0x0", "max_price_per_unit": "0xe8d4a51000"},
    "l2_gas": {"max_amount": "0x2000000", "max_price_per_unit": "0x2cb417800"},
    "l1_data_gas": {"max_amount": "0x1b0", "max_price_per_unit": "0x5dc"},
}


# ── Request / Response models ────────────────────────────


class FundRequest(BaseModel):
    account_address: str


class DeployAccountRequest(BaseModel):
    session_id: str
    public_key: str
    account_address: str


class DeployCounterRequest(BaseModel):
    session_id: str


class InvokeRequest(BaseModel):
    session_id: str
    amount: int = 1
    signature_r: str
    signature_s: str
    nonce: int


class ProveRequest(BaseModel):
    session_id: str
    tx_hash: str
    block_number: int


class SubmitProofRequest(BaseModel):
    session_id: str


class ReadCounterRequest(BaseModel):
    contract_address: str


# ── Helpers ──────────────────────────────────────────────


def rpc_call_raw(payload: dict) -> dict:
    """Send a raw JSON-RPC payload to the Starknet node."""
    import urllib.request

    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        master.rpc_url,
        data=data,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def rpc_call(method: str, params: dict) -> dict:
    """Make a JSON-RPC call to the Starknet node."""
    return rpc_call_raw(
        {"jsonrpc": "2.0", "method": method, "params": params, "id": 1}
    )


def _receipt_block_number(receipt: dict) -> int | None:
    """Extract block number from a receipt, handling both hex and int."""
    bn = receipt.get("block_number")
    if bn is None:
        return None
    return int(bn, 16) if isinstance(bn, str) and bn.startswith("0x") else int(bn)


async def wait_for_tx(tx_hash: str, timeout: int = 120, poll_interval: float = 2.0) -> dict:
    """Poll for a transaction receipt until it is in a finalized block.

    We require both ACCEPTED_ON_L2 status AND a block_number in the receipt.
    A tx can be ACCEPTED_ON_L2 while still in the pending block (no block_number),
    so waiting for block_number guarantees the block is closed.
    """
    import time

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
                has_block = receipt.get("block_number") is not None
                if status in ("ACCEPTED_ON_L2", "ACCEPTED_ON_L1") and has_block:
                    return receipt
        except Exception:
            pass
        await asyncio.sleep(poll_interval)
    raise TimeoutError(f"Tx {tx_hash} not confirmed within {timeout}s")


async def wait_for_block_after(block_number: int, timeout: int = 120, poll_interval: float = 2.0) -> int:
    """Wait until the chain head advances past the given block number.

    This ensures the block is fully closed and its state is committed,
    so the next submitted tx will land in a strictly later block.
    """
    import time

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = rpc_call("starknet_blockNumber", {})
            if "result" in result:
                current = result["result"]
                if current > block_number:
                    return current
        except Exception:
            pass
        await asyncio.sleep(poll_interval)
    raise TimeoutError(f"No new block after {block_number} within {timeout}s")


def get_session(session_id: str) -> dict:
    if session_id not in sessions:
        sessions[session_id] = {}
    return sessions[session_id]


# ── Endpoints ────────────────────────────────────────────


@app.get("/api/health")
async def health():
    return {"status": "ok", "rpc_url": master.rpc_url}


@app.post("/api/fund")
async def fund_account(req: FundRequest):
    """Transfer STRK from master account to a newly generated account."""
    try:
        tx_hash = await master.transfer_strk(req.account_address, amount_wei=10 * 10**18)
        log.info(f"Fund tx submitted: {tx_hash}")
        receipt = await wait_for_tx(tx_hash)
        bn = _receipt_block_number(receipt)
        log.info(f"Fund tx confirmed in block {bn}")
        if bn is not None:
            next_bn = await wait_for_block_after(bn)
            log.info(f"Block advanced to {next_bn} after fund")
        return {
            "tx_hash": tx_hash,
            "amount": "10 STRK",
            "block_number": bn,
        }
    except TimeoutError as e:
        raise HTTPException(status_code=504, detail=str(e))
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.post("/api/deploy-account")
async def deploy_account(req: DeployAccountRequest):
    """Deploy an OpenZeppelin account contract for the generated key."""
    session = get_session(req.session_id)
    try:
        result = await master.deploy_oz_account(req.public_key, req.account_address)
        if result.get("tx_hash"):
            log.info(f"Deploy account tx: {result['tx_hash']}")
            receipt = await wait_for_tx(result["tx_hash"])
            bn = _receipt_block_number(receipt)
            log.info(f"Deploy account confirmed in block {bn}")
            result["block_number"] = bn
            if bn is not None:
                await wait_for_block_after(bn)
        session["account_address"] = req.account_address
        session["account_deployed"] = True
        return result
    except TimeoutError as e:
        raise HTTPException(status_code=504, detail=str(e))
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.post("/api/deploy-counter")
async def deploy_counter(req: DeployCounterRequest):
    """Declare + deploy the Counter contract (funded by master account)."""
    session = get_session(req.session_id)
    try:
        result = await master.deploy_counter(
            str(TESTS_DIR / "contracts" / "target" / "dev")
        )
        if result.get("tx_hash"):
            log.info(f"Deploy counter tx: {result['tx_hash']}")
            receipt = await wait_for_tx(result["tx_hash"])
            bn = _receipt_block_number(receipt)
            log.info(f"Deploy counter confirmed in block {bn}")
            result["block_number"] = bn
            if bn is not None:
                await wait_for_block_after(bn)
        session["contract_address"] = result["contract_address"]
        session["class_hash"] = result["class_hash"]
        return result
    except TimeoutError as e:
        raise HTTPException(status_code=504, detail=str(e))
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.post("/api/read-counter")
async def read_counter(req: ReadCounterRequest):
    """Call get_counter() on the deployed contract."""
    try:
        result = rpc_call(
            "starknet_call",
            {
                "request": {
                    "contract_address": req.contract_address,
                    "entry_point_selector": GET_COUNTER_SELECTOR,
                    "calldata": [],
                },
                "block_id": "latest",
            },
        )
        value = int(result["result"][0], 16) if result.get("result") else 0
        return {"counter_value": value}
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/api/nonce/{account_address}")
async def get_nonce(account_address: str):
    """Fetch the current nonce for an account."""
    try:
        result = rpc_call(
            "starknet_getNonce",
            {"block_id": "latest", "contract_address": account_address},
        )
        if "error" in result:
            raise HTTPException(status_code=400, detail=json.dumps(result["error"]))
        nonce = int(result["result"], 16)
        return {"nonce": nonce, "nonce_hex": result["result"]}
    except HTTPException:
        raise
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.post("/api/invoke")
async def invoke_increment(req: InvokeRequest):
    """Submit a pre-signed increment() invoke transaction."""
    session = get_session(req.session_id)
    contract_address = session.get("contract_address")
    if not contract_address:
        raise HTTPException(status_code=400, detail="No counter contract deployed")

    account_address = session.get("account_address")
    if not account_address:
        raise HTTPException(status_code=400, detail="No account deployed")

    # Build multicall calldata: [num_calls, to, selector, calldata_len, ...calldata]
    calldata = [
        "0x1",
        contract_address,
        INCREMENT_SELECTOR,
        "0x1",
        hex(req.amount),
    ]

    payload = {
        "jsonrpc": "2.0",
        "method": "starknet_addInvokeTransaction",
        "params": {
            "invoke_transaction": {
                "type": "INVOKE",
                "version": "0x3",
                "sender_address": account_address,
                "calldata": calldata,
                "nonce": hex(req.nonce),
                "resource_bounds": RESOURCE_BOUNDS_FOR_RPC,
                "tip": "0x0",
                "paymaster_data": [],
                "account_deployment_data": [],
                "nonce_data_availability_mode": "L1",
                "fee_data_availability_mode": "L1",
                "signature": [req.signature_r, req.signature_s],
            }
        },
        "id": 1,
    }

    result = rpc_call_raw(payload)

    if "error" in result:
        raise HTTPException(status_code=400, detail=json.dumps(result["error"]))

    if "result" not in result:
        raise HTTPException(status_code=500, detail=f"Unexpected RPC response: {json.dumps(result)}")

    tx_hash = result["result"]["transaction_hash"]
    session["last_invoke_tx"] = tx_hash
    log.info(f"Invoke tx submitted: {tx_hash}")

    # Wait for tx inclusion so the prover can reference the correct block
    try:
        receipt = await wait_for_tx(tx_hash)
        bn = _receipt_block_number(receipt)
        log.info(f"Invoke tx confirmed in block {bn}")
        if bn is not None:
            session["invoke_block"] = bn
        return {"tx_hash": tx_hash, "block_number": bn}
    except TimeoutError:
        return {"tx_hash": tx_hash, "block_number": None, "warning": "Tx submitted but not yet confirmed"}


@app.get("/api/prove/{session_id}")
async def prove_transaction(session_id: str):
    """Run virtual OS + stwo prover. Returns SSE stream of log lines."""
    session = get_session(session_id)
    tx_hash = session.get("last_invoke_tx")
    if not tx_hash:
        raise HTTPException(status_code=400, detail="No invoke tx to prove")

    contract_address = session.get("contract_address")

    async def event_stream():
        OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
        proof_output = OUTPUT_DIR / f"{session_id}.proof"

        yield {"event": "log", "data": f"Starting proof generation for {tx_hash}..."}

        # Get the block number from the invoke receipt (already waited in /api/invoke)
        invoke_block = session.get("invoke_block")
        if not invoke_block:
            yield {"event": "log", "data": "Waiting for tx inclusion..."}
            try:
                receipt = await wait_for_tx(tx_hash)
                bn = receipt.get("block_number")
                invoke_block = int(bn, 16) if isinstance(bn, str) else bn
            except TimeoutError:
                yield {"event": "error", "data": "Tx not included in time"}
                return

        if not invoke_block:
            yield {"event": "error", "data": "Could not determine block number"}
            return

        prove_block = invoke_block - 1
        yield {
            "event": "log",
            "data": f"Tx included in block {invoke_block}. Proving against block {prove_block}...",
        }
        session["prove_block"] = prove_block

        # Run virtual OS + prover
        run_script = str(SCRIPTS_DIR / "run-virtual-os.sh")
        env = {
            **os.environ,
            "STARKNET_RPC_URL": master.rpc_url,
            "STARKNET_ACCOUNT_ADDRESS": master.address,
            "STARKNET_PRIVATE_KEY": master.private_key,
        }

        proc = await asyncio.create_subprocess_exec(
            run_script,
            "--block-number",
            str(prove_block),
            "--tx-hash",
            tx_hash,
            "--rpc-url",
            master.rpc_url,
            "--output",
            str(proof_output),
            "--strk-fee-token",
            master.STRK_TOKEN,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=env,
        )

        async for line in proc.stdout:
            text = line.decode().rstrip()
            if text:
                yield {"event": "log", "data": text}

        await proc.wait()

        if proof_output.exists():
            proof_size = proof_output.stat().st_size
            session["proof_file"] = str(proof_output)
            yield {
                "event": "complete",
                "data": json.dumps(
                    {"proof_size": proof_size, "proof_file": str(proof_output)}
                ),
            }
        else:
            yield {"event": "error", "data": "Proof generation failed"}

    return EventSourceResponse(event_stream())


@app.post("/api/submit-proof")
async def submit_proof(req: SubmitProofRequest):
    """Sign and submit the proof-bearing transaction to the gateway."""
    session = get_session(req.session_id)
    proof_file = session.get("proof_file")
    if not proof_file or not Path(proof_file).exists():
        raise HTTPException(status_code=400, detail="No proof file available")

    contract_address = session.get("contract_address")
    proof_facts_file = proof_file.replace(".proof", ".proof_facts")

    # Build calldata for increment(1) — must match the selector computed at startup
    calldata_csv = f"0x1,{contract_address},{INCREMENT_SELECTOR},0x1,0x1"

    env = {
        **os.environ,
        "STARKNET_RPC_URL": master.rpc_url,
        "STARKNET_ACCOUNT_ADDRESS": master.address,
        "STARKNET_PRIVATE_KEY": master.private_key,
        "STARKNET_GATEWAY_URL": os.environ.get(
            "STARKNET_GATEWAY_URL",
            "https://privacy-starknet-integration.starknet.io",
        ),
    }

    proc = subprocess.run(
        [
            "python3.11",
            str(TESTS_DIR / "sign-and-submit.py"),
            proof_file,
            proof_facts_file,
            calldata_csv,
            contract_address,
        ],
        capture_output=True,
        text=True,
        env=env,
        timeout=120,
    )

    if proc.returncode != 0:
        raise HTTPException(
            status_code=500,
            detail=f"Submission failed: {proc.stderr or proc.stdout}",
        )

    # Parse tx hash from output
    for line in proc.stdout.split("\n"):
        if "tx_hash" in line.lower() and "0x" in line:
            match = re.search(r"0x[0-9a-fA-F]+", line)
            if match:
                return {"tx_hash": match.group(), "output": proc.stdout}

    return {"output": proc.stdout}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=8080)
