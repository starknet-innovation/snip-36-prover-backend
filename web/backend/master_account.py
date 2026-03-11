"""Master account helper — handles funding and contract deployment for the playground.

Uses sncast for all on-chain operations (signing, declaring, deploying, invoking).
The master account must be imported into sncast as "playground-master" before use
(see web/setup-master.sh).
"""

import re
import secrets
import subprocess


def _parse_hex(key: str, text: str) -> str | None:
    """Extract a hex value after a given key from sncast output.

    Matches flexibly: underscores and spaces are treated as equivalent,
    so 'transaction_hash' matches 'Transaction Hash'.
    """
    # Normalize key: replace underscores with a regex pattern matching either
    pattern = re.compile(key.replace("_", "[_ ]"), re.IGNORECASE)
    for line in text.split("\n"):
        if pattern.search(line):
            m = re.search(r"0x[0-9a-fA-F]+", line)
            if m:
                return m.group()
    return None


class MasterAccount:
    """Pre-funded account that funds dev accounts and deploys contracts."""

    # STRK token on integration sepolia
    STRK_TOKEN = (
        "0x70a5da4f557b77a9c54546e4bcc900806e28793d8e3eaaa207428d2387249b7"
    )

    def __init__(self, rpc_url: str, address: str, private_key: str):
        self.rpc_url = rpc_url
        self.address = address
        self.private_key = private_key

    def _sncast(self, *args: str, cwd: str | None = None) -> subprocess.CompletedProcess:
        """Run an sncast command with the playground-master account."""
        cmd = ["sncast", "--account", "playground-master", *args, "--url", self.rpc_url]
        return subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)

    async def transfer_strk(self, to_address: str, amount_wei: int = 10**18) -> str:
        """Transfer STRK tokens to a target address."""
        amount_low = hex(amount_wei & ((1 << 128) - 1))
        amount_high = hex(amount_wei >> 128)

        result = self._sncast(
            "invoke",
            "--contract-address", self.STRK_TOKEN,
            "--function", "transfer",
            "--calldata", f"{to_address} {amount_low} {amount_high}",
        )

        tx_hash = _parse_hex("transaction_hash", result.stdout)
        if not tx_hash:
            raise RuntimeError(f"Transfer failed: {result.stderr or result.stdout}")
        return tx_hash

    async def deploy_oz_account(self, public_key: str, expected_address: str) -> dict:
        """Deploy an OpenZeppelin account contract for the generated key."""
        # OZ Account class hash on integration sepolia
        oz_class_hash = "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564"

        result = self._sncast(
            "deploy",
            "--class-hash", oz_class_hash,
            "--constructor-calldata", public_key,
            "--salt", public_key,
        )

        address = _parse_hex("contract_address", result.stdout)
        tx_hash = _parse_hex("transaction_hash", result.stdout)

        if not address:
            raise RuntimeError(f"Account deploy failed: {result.stderr or result.stdout}")

        return {"account_address": address, "tx_hash": tx_hash}

    async def deploy_counter(self, artifacts_dir: str) -> dict:
        """Declare + deploy the Counter contract."""
        import os

        # Declare
        declare_result = self._sncast(
            "declare",
            "--contract-name", "Counter",
            cwd=os.path.dirname(artifacts_dir),
        )

        # Extract class hash (long hex string)
        class_hash = None
        for line in (declare_result.stdout + declare_result.stderr).split("\n"):
            m = re.search(r"0x[0-9a-fA-F]{50,}", line)
            if m:
                class_hash = m.group()
                break

        if not class_hash:
            raise RuntimeError(
                f"Declare failed: {declare_result.stderr or declare_result.stdout}"
            )

        # Deploy with random salt
        salt = "0x" + secrets.token_hex(16)
        deploy_result = self._sncast(
            "deploy",
            "--class-hash", class_hash,
            "--salt", salt,
        )

        contract_address = _parse_hex("contract_address", deploy_result.stdout)
        tx_hash = _parse_hex("transaction_hash", deploy_result.stdout)

        if not contract_address:
            raise RuntimeError(
                f"Deploy failed: {deploy_result.stderr or deploy_result.stdout}"
            )

        return {
            "class_hash": class_hash,
            "contract_address": contract_address,
            "tx_hash": tx_hash,
        }
