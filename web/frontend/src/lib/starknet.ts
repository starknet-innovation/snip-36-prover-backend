/**
 * Starknet key generation, signing, and transaction hashing.
 *
 * Uses starknet.js v9 in-browser — no wallet extension needed.
 *
 * For standard invokes (step 5), we use starknet.js's built-in hash.
 * For SNIP-36 proof-bearing txs (step 7), the backend computes the
 * custom hash (3 resource bounds + proof_facts) via sign-and-submit.py.
 */

import { ec, hash, encode, stark, shortString } from "starknet";

// ── Constants ───────────────────────────────────────────

/** OZ Account class hash on sepolia. */
const OZ_ACCOUNT_CLASS_HASH =
  "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564";

/**
 * Chain ID for SN_SEPOLIA.
 * Not a built-in starknet.js constant — encode manually.
 */
export const SN_SEPOLIA_CHAIN_ID =
  shortString.encodeShortString("SN_SEPOLIA");

/**
 * Resource bounds for playground transactions.
 * Must match the backend's RESOURCE_BOUNDS_FOR_RPC.
 *
 * starknet.js v9.4+ includes all 3 resource types in the hash
 * (L1_GAS, L2_GAS, L1_DATA_GAS) per the latest Starknet spec.
 */
/**
 * starknet.js v9.4.2 expects all resource bound values as BigInt
 * (no auto-conversion from strings in the hash internals).
 */
export const RESOURCE_BOUNDS = {
  l1_gas: { max_amount: 0x0n, max_price_per_unit: 0xe8d4a51000n },
  l2_gas: { max_amount: 0x2000000n, max_price_per_unit: 0x2cb417800n },
  l1_data_gas: { max_amount: 0x1b0n, max_price_per_unit: 0x5dcn },
} as const;

// ── Key Generation ──────────────────────────────────────

export interface StarkKeyPair {
  privateKey: string;
  publicKey: string;
  accountAddress: string;
}

/**
 * Generate a random Stark key pair and compute the expected OZ account address.
 */
export function generateKeyPair(): StarkKeyPair {
  const privateKey = stark.randomAddress();
  const publicKey = ec.starkCurve.getStarkKey(privateKey);

  const accountAddress = hash.calculateContractAddressFromHash(
    publicKey,
    OZ_ACCOUNT_CLASS_HASH,
    [publicKey],
    0
  );

  return {
    privateKey,
    publicKey,
    accountAddress: encode.addHexPrefix(
      encode.removeHexPrefix(accountAddress).padStart(64, "0")
    ),
  };
}

// ── Selectors ───────────────────────────────────────────

/** Pre-computed selectors for the Counter contract ABI. */
export const SELECTORS = {
  increment: hash.getSelectorFromName("increment"),
  get_counter: hash.getSelectorFromName("get_counter"),
} as const;

// ── Transaction Hash (Invoke V3) ────────────────────────

export interface InvokeV3TxHashParams {
  senderAddress: string;
  calldata: string[];
  nonce: number;
}

/**
 * Compute an invoke v3 transaction hash using starknet.js.
 *
 * This uses the standard Starknet hash (L1_GAS + L2_GAS only).
 * For SNIP-36 proof-bearing transactions (which include L1_DATA_GAS
 * and proof_facts in the hash), the backend handles the custom computation.
 */
export function computeInvokeV3TxHash(params: InvokeV3TxHashParams): string {
  return hash.calculateInvokeTransactionHash({
    senderAddress: params.senderAddress,
    version: "0x3",
    compiledCalldata: params.calldata,
    chainId: SN_SEPOLIA_CHAIN_ID as any,
    nonce: "0x" + params.nonce.toString(16),
    accountDeploymentData: [],
    nonceDataAvailabilityMode: "0x0" as any, // L1
    feeDataAvailabilityMode: "0x0" as any, // L1
    resourceBounds: RESOURCE_BOUNDS,
    tip: "0x0",
    paymasterData: [],
  });
}

// ── Signing ─────────────────────────────────────────────

/**
 * Sign a transaction hash with the given private key.
 * Returns r, s as hex strings.
 */
export function signTransaction(
  txHash: string,
  privateKey: string
): { r: string; s: string } {
  const sig = ec.starkCurve.sign(
    encode.removeHexPrefix(txHash),
    encode.removeHexPrefix(privateKey)
  );
  return {
    r: encode.addHexPrefix(sig.r.toString(16)),
    s: encode.addHexPrefix(sig.s.toString(16)),
  };
}

// ── Display Helpers ─────────────────────────────────────

/** Truncate a hex string for display. */
export function truncateHex(hex: string, chars = 6): string {
  if (hex.length <= chars * 2 + 4) return hex;
  return `${hex.slice(0, chars + 2)}...${hex.slice(-chars)}`;
}
