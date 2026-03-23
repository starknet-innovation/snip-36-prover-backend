//! SNIP-36 transaction hash computation and ECDSA signing.
//!
//! The standard Starknet v3 invoke transaction hash is computed per SNIP-8.
//! SNIP-36 extends this by appending a `proof_facts_hash` to the Poseidon
//! hash chain when proof_facts are present.
//!
//! Standard SDKs (starknet-py, starknet.js) do NOT include proof_facts — this
//! module is the canonical Rust implementation of the SNIP-36 hash extension.

use starknet_crypto::poseidon_hash_many;
use starknet_types_core::felt::Felt;

use crate::types::ResourceBounds;

/// "invoke" encoded as a short string felt.
fn invoke_prefix() -> Felt {
    Felt::from_bytes_be_slice(b"invoke")
}

/// Encode a chain ID string (e.g. "SN_SEPOLIA") as a felt.
pub fn chain_id_felt(chain_id: &str) -> Felt {
    Felt::from_bytes_be_slice(chain_id.as_bytes())
}

/// Convert a resource name string to a 7-byte felt value.
///
/// Names shorter than 7 bytes are right-aligned (big-endian).
/// Names longer than 7 bytes are truncated to 7.
fn resource_name_felt(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let len = bytes.len().min(7);
    let mut buf = [0u8; 8];
    buf[8 - len..].copy_from_slice(&bytes[..len]);
    u64::from_be_bytes(buf)
}

/// Pack resource bounds into a single felt:
/// `[0(1 byte) | resource_name(7 bytes) | max_amount(8 bytes) | max_price(16 bytes)]`
fn concat_resource(max_amount: u64, max_price: u128, resource_name: &str) -> Felt {
    let name = resource_name_felt(resource_name) as u128;
    // Build a 256-bit value: name at bits [248:192], amount at [191:128], price at [127:0]
    // name (56 bits) << 192 | max_amount (64 bits) << 128 | max_price (128 bits)
    let high = (name << 64) | (max_amount as u128);
    let low = max_price;

    // Construct from two u128 halves: value = high * 2^128 + low
    let high_felt = Felt::from(high);
    let shift = Felt::from(1u128 << 64).pow_felt(&Felt::TWO); // 2^128
    high_felt * shift + Felt::from(low)
}

/// Compute the `tip_resource_bounds_hash` per SNIP-8.
///
/// ```text
/// poseidon(tip, concat(L1_GAS), concat(L2_GAS), concat(L1_DATA_GAS))
/// ```
fn compute_tip_resource_bounds_hash(tip: Felt, bounds: &ResourceBounds) -> Felt {
    let l1 = concat_resource(
        bounds.l1_gas.max_amount,
        bounds.l1_gas.max_price_per_unit,
        "L1_GAS",
    );
    let l2 = concat_resource(
        bounds.l2_gas.max_amount,
        bounds.l2_gas.max_price_per_unit,
        "L2_GAS",
    );
    let l1_data = concat_resource(
        bounds.l1_data_gas.max_amount,
        bounds.l1_data_gas.max_price_per_unit,
        "L1_DATA",
    );
    poseidon_hash_many(&[tip, l1, l2, l1_data])
}

/// Compute the invoke v3 transaction hash, optionally including proof_facts (SNIP-36).
///
/// The hash chain is:
/// ```text
/// poseidon(
///     INVOKE_PREFIX,
///     version,
///     sender_address,
///     tip_resource_bounds_hash,
///     paymaster_data_hash,
///     chain_id,
///     nonce,
///     da_mode,
///     account_deployment_data_hash,
///     calldata_hash,
///     [proof_facts_hash]   // only if proof_facts is non-empty
/// )
/// ```
#[allow(clippy::too_many_arguments)]
pub fn compute_invoke_v3_tx_hash(
    sender_address: Felt,
    calldata: &[Felt],
    chain_id: Felt,
    nonce: Felt,
    tip: Felt,
    resource_bounds: &ResourceBounds,
    paymaster_data: &[Felt],
    account_deployment_data: &[Felt],
    nonce_da_mode: u32,
    fee_da_mode: u32,
    proof_facts: &[Felt],
) -> Felt {
    let tip_rb_hash = compute_tip_resource_bounds_hash(tip, resource_bounds);

    let paymaster_data_hash = poseidon_hash_many(paymaster_data);
    let account_deployment_data_hash = poseidon_hash_many(account_deployment_data);
    let calldata_hash = poseidon_hash_many(calldata);

    // data_availability_mode: [0..0 (192 bit) | nonce_mode (32 bit) | fee_mode (32 bit)]
    let da_mode = Felt::from(((nonce_da_mode as u64) << 32) | fee_da_mode as u64);

    let version = Felt::THREE;

    let mut elements = vec![
        invoke_prefix(),
        version,
        sender_address,
        tip_rb_hash,
        paymaster_data_hash,
        chain_id,
        nonce,
        da_mode,
        account_deployment_data_hash,
        calldata_hash,
    ];

    // SNIP-36 extension: append proof_facts_hash if non-empty
    if !proof_facts.is_empty() {
        let proof_facts_hash = poseidon_hash_many(proof_facts);
        elements.push(proof_facts_hash);
    }

    poseidon_hash_many(&elements)
}

/// ECDSA signature (r, s) over the Stark curve.
#[derive(Debug, Clone)]
pub struct Signature {
    pub r: Felt,
    pub s: Felt,
}

/// Sign a message hash with an ECDSA private key on the Stark curve.
///
/// Uses RFC-6979 deterministic nonce generation to avoid nonce reuse,
/// which would leak the private key.
pub fn sign(private_key: Felt, message_hash: Felt) -> Result<Signature, SignError> {
    let k = starknet_crypto::rfc6979_generate_k(&message_hash, &private_key, None);
    let sig = starknet_crypto::sign(&private_key, &message_hash, &k)
        .map_err(|e| SignError::Ecdsa(e.to_string()))?;
    Ok(Signature { r: sig.r, s: sig.s })
}

/// Compute the SNIP-36 tx hash, sign it, and return the RPC invoke transaction payload.
pub fn sign_and_build_payload(
    params: &crate::types::SubmitParams,
) -> Result<(Felt, serde_json::Value), SignError> {
    let tx_hash = compute_invoke_v3_tx_hash(
        params.sender_address,
        &params.calldata,
        params.chain_id,
        params.nonce,
        Felt::ZERO, // tip
        &params.resource_bounds,
        &[],  // paymaster_data
        &[],  // account_deployment_data
        0,    // nonce_da_mode (L1)
        0,    // fee_da_mode (L1)
        &params.proof_facts,
    );

    let sig = sign(params.private_key, tx_hash)?;

    let calldata_hex: Vec<String> = params
        .calldata
        .iter()
        .map(|f| format!("{:#x}", f))
        .collect();

    let proof_facts_hex: Vec<String> = params
        .proof_facts
        .iter()
        .map(|f| format!("{:#x}", f))
        .collect();

    let payload = serde_json::json!({
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": format!("{:#x}", params.sender_address),
        "calldata": calldata_hex,
        "nonce": format!("{:#x}", params.nonce),
        "resource_bounds": params.resource_bounds.to_rpc_json(),
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "signature": [format!("{:#x}", sig.r), format!("{:#x}", sig.s)],
        "proof": params.proof_base64,
        "proof_facts": proof_facts_hex,
    });

    Ok((tx_hash, payload))
}

#[derive(Debug, thiserror::Error)]
pub enum SignError {
    #[error("ECDSA signing failed: {0}")]
    Ecdsa(String),
}

/// Parse a hex string (with or without 0x prefix) to a Felt.
pub fn felt_from_hex(hex_str: &str) -> Result<Felt, String> {
    Felt::from_hex(hex_str).map_err(|e| format!("invalid felt hex '{hex_str}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoke_prefix() {
        // "invoke" = 0x696e766f6b65
        let prefix = invoke_prefix();
        assert_eq!(format!("{:#x}", prefix), "0x696e766f6b65");
    }

    #[test]
    fn test_resource_name_felt() {
        assert_eq!(resource_name_felt("L1_GAS"), 0x004c315f474153);
        assert_eq!(resource_name_felt("L2_GAS"), 0x004c325f474153);
        // "L1_DATA" is exactly 7 bytes
        assert_eq!(resource_name_felt("L1_DATA"), 0x4c315f44415441);
    }

    #[test]
    fn test_chain_id_encoding() {
        let cid = chain_id_felt("SN_SEPOLIA");
        // Should match Python: int.from_bytes(b"SN_SEPOLIA", "big")
        let expected = Felt::from_hex(
            "0x534e5f5345504f4c4941",
        )
        .unwrap();
        assert_eq!(cid, expected);
    }

    #[test]
    fn test_tx_hash_without_proof_facts() {
        // Ensure the hash chain produces a deterministic result with empty proof_facts.
        let sender = Felt::from_hex("0x123").unwrap();
        let calldata = vec![Felt::ONE];
        let chain_id = chain_id_felt("SN_SEPOLIA");
        let nonce = Felt::ZERO;
        let bounds = ResourceBounds::default();

        let h1 = compute_invoke_v3_tx_hash(
            sender, &calldata, chain_id, nonce, Felt::ZERO, &bounds, &[], &[], 0, 0, &[],
        );
        let h2 = compute_invoke_v3_tx_hash(
            sender, &calldata, chain_id, nonce, Felt::ZERO, &bounds, &[], &[], 0, 0, &[],
        );
        assert_eq!(h1, h2, "hash should be deterministic");
    }

    #[test]
    fn test_tx_hash_with_proof_facts_differs() {
        let sender = Felt::from_hex("0x123").unwrap();
        let calldata = vec![Felt::ONE];
        let chain_id = chain_id_felt("SN_SEPOLIA");
        let nonce = Felt::ZERO;
        let bounds = ResourceBounds::default();

        let h_without = compute_invoke_v3_tx_hash(
            sender, &calldata, chain_id, nonce, Felt::ZERO, &bounds, &[], &[], 0, 0, &[],
        );
        let proof_facts = vec![Felt::from_hex("0x50524f4f4630").unwrap()];
        let h_with = compute_invoke_v3_tx_hash(
            sender,
            &calldata,
            chain_id,
            nonce,
            Felt::ZERO,
            &bounds,
            &[],
            &[],
            0,
            0,
            &proof_facts,
        );
        assert_ne!(
            h_without, h_with,
            "hash with proof_facts should differ from without"
        );
    }
}
