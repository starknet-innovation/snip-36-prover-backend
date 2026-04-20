#!/usr/bin/env python3
"""One-off: counterfactual-deploy the OZ account for .env.mainnet.

Reads STARKNET_RPC_URL / STARKNET_PRIVATE_KEY from .env.mainnet, confirms the
account is funded, sends a v3 deploy_account, waits for acceptance, and prints
the resulting address.
"""

import asyncio
import os
import sys
from pathlib import Path

from starknet_py.hash.address import compute_address
from starknet_py.hash.utils import private_to_stark_key
from starknet_py.net.account.account import Account
from starknet_py.net.client_models import Call
from starknet_py.net.full_node_client import FullNodeClient
from starknet_py.net.models import StarknetChainId
from starknet_py.net.signer.stark_curve_signer import KeyPair, StarkCurveSigner

OZ_CLASS_HASH = 0x05B4B537EAA2399E3AA99C4E2E0208EBD6C71BC1467938CD52C798C601E43564

STRK = 0x04718F5A0FC34CC1AF16A1CDEE98FFB20C31F5CD61D6AB07201858F4287C938D
BALANCE_OF = 0x035A73CD311A05D46DEDA634C5EE045DB92F811B4E74BCA4437FCB5302B7AF33


def load_env(path: Path) -> dict:
    env = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            continue
        k, v = line.split("=", 1)
        env[k.strip()] = v.strip().strip('"').strip("'")
    return env


async def main():
    env_path = Path(sys.argv[1] if len(sys.argv) > 1 else ".env.mainnet")
    env = load_env(env_path)

    rpc_url = env["STARKNET_RPC_URL"]
    priv = int(env["STARKNET_PRIVATE_KEY"], 16)
    pub = private_to_stark_key(priv)

    address = compute_address(
        salt=pub,
        class_hash=OZ_CLASS_HASH,
        constructor_calldata=[pub],
        deployer_address=0,
    )
    print(f"public_key: {hex(pub)}")
    print(f"address:    {hex(address)}")

    client = FullNodeClient(node_url=rpc_url)

    balance = await client.call_contract(
        call=Call(to_addr=STRK, selector=BALANCE_OF, calldata=[address]),
        block_number="latest",
    )
    low, high = balance
    strk = low + (high << 128)
    print(f"STRK balance: {strk / 1e18:.4f}")
    if strk == 0:
        print("account not funded yet; aborting")
        sys.exit(1)

    # Try to detect already-deployed
    try:
        cls_hash = await client.get_class_hash_at(contract_address=address, block_number="latest")
        print(f"account already deployed, class_hash={hex(cls_hash)}")
        return
    except Exception:
        pass  # not deployed — proceed

    signer = StarkCurveSigner(
        account_address=address,
        key_pair=KeyPair(private_key=priv, public_key=pub),
        chain_id=StarknetChainId.MAINNET,
    )
    account = Account(
        address=address,
        client=client,
        signer=signer,
        chain=StarknetChainId.MAINNET,
    )

    print("sending deploy_account (v3, STRK fees)...")
    deploy_result = await Account.deploy_account_v3(
        address=address,
        class_hash=OZ_CLASS_HASH,
        salt=pub,
        key_pair=KeyPair(private_key=priv, public_key=pub),
        client=client,
        constructor_calldata=[pub],
        auto_estimate=True,
    )
    print(f"tx hash: {hex(deploy_result.hash)}")
    print("waiting for acceptance...")
    await deploy_result.wait_for_acceptance()
    print(f"deployed at {hex(address)}")


if __name__ == "__main__":
    asyncio.run(main())
