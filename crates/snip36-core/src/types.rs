use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

/// Resource bounds for a single gas type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBound {
    pub max_amount: u64,
    pub max_price_per_unit: u128,
}

/// All three resource bound types for a v3 invoke transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceBounds {
    pub l1_gas: ResourceBound,
    pub l2_gas: ResourceBound,
    pub l1_data_gas: ResourceBound,
}

impl ResourceBounds {
    /// Build resource bounds from live gas prices with a 2x safety multiplier.
    pub fn from_prices(l1_gas_price: u128, l1_data_gas_price: u128, l2_gas_price: u128) -> Self {
        Self {
            l1_gas: ResourceBound {
                max_amount: 0x10000,
                max_price_per_unit: l1_gas_price.saturating_mul(2),
            },
            l2_gas: ResourceBound {
                max_amount: 0x7000000,
                max_price_per_unit: l2_gas_price.saturating_mul(2),
            },
            l1_data_gas: ResourceBound {
                max_amount: 0x1b0,
                max_price_per_unit: l1_data_gas_price.saturating_mul(2),
            },
        }
    }

    /// Resource bounds for virtual OS proving: enough gas for execution, zero price (no fee).
    pub fn zero_fee() -> Self {
        Self {
            l1_gas: ResourceBound {
                max_amount: 0,
                max_price_per_unit: 0,
            },
            l2_gas: ResourceBound {
                max_amount: 0x7000000,
                max_price_per_unit: 0,
            },
            l1_data_gas: ResourceBound {
                max_amount: 0x1b0,
                max_price_per_unit: 0,
            },
        }
    }

    /// Format for the Starknet JSON-RPC `resource_bounds` field (lowercase keys).
    pub fn to_rpc_json(&self) -> serde_json::Value {
        serde_json::json!({
            "l1_gas": {
                "max_amount": format!("{:#x}", self.l1_gas.max_amount),
                "max_price_per_unit": format!("{:#x}", self.l1_gas.max_price_per_unit),
            },
            "l2_gas": {
                "max_amount": format!("{:#x}", self.l2_gas.max_amount),
                "max_price_per_unit": format!("{:#x}", self.l2_gas.max_price_per_unit),
            },
            "l1_data_gas": {
                "max_amount": format!("{:#x}", self.l1_data_gas.max_amount),
                "max_price_per_unit": format!("{:#x}", self.l1_data_gas.max_price_per_unit),
            },
        })
    }

    /// Format for the Starknet gateway `resource_bounds` field (uppercase keys).
    pub fn to_gateway_json(&self) -> serde_json::Value {
        serde_json::json!({
            "L1_GAS": {
                "max_amount": format!("{:#x}", self.l1_gas.max_amount),
                "max_price_per_unit": format!("{:#x}", self.l1_gas.max_price_per_unit),
            },
            "L2_GAS": {
                "max_amount": format!("{:#x}", self.l2_gas.max_amount),
                "max_price_per_unit": format!("{:#x}", self.l2_gas.max_price_per_unit),
            },
            "L1_DATA_GAS": {
                "max_amount": format!("{:#x}", self.l1_data_gas.max_amount),
                "max_price_per_unit": format!("{:#x}", self.l1_data_gas.max_price_per_unit),
            },
        })
    }
}

/// Proof data returned by the prover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOutput {
    /// Base64-encoded proof bytes.
    pub proof_base64: String,
    /// Array of hex-encoded field elements (proof facts).
    pub proof_facts: Vec<String>,
}

/// Parameters for a SNIP-36 proof submission via RPC.
#[derive(Debug, Clone)]
pub struct SubmitParams {
    pub sender_address: Felt,
    pub private_key: Felt,
    pub calldata: Vec<Felt>,
    pub proof_base64: String,
    pub proof_facts: Vec<Felt>,
    pub nonce: Felt,
    pub chain_id: Felt,
    pub resource_bounds: ResourceBounds,
}

/// Session state for the playground web UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub account_address: Option<String>,
    pub account_deployed: bool,
    pub contract_address: Option<String>,
    pub class_hash: Option<String>,
    pub last_invoke_tx: Option<String>,
    pub invoke_block: Option<u64>,
    pub prove_block: Option<u64>,
    pub proof_file: Option<String>,
    /// Block number where the application contract was deployed.
    pub deploy_block: Option<u64>,
    /// Reference block for the next prove (updated after each proven tx lands).
    pub last_reference_block: Option<u64>,
}

// Well-known constants

/// STRK token address on sepolia.
pub const STRK_TOKEN: &str = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

/// OpenZeppelin Account class hash on sepolia.
pub const OZ_ACCOUNT_CLASS_HASH: &str =
    "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564";

/// ERC-20 `balance_of(account)` selector — standard across all Starknet tokens.
pub const BALANCE_OF_SELECTOR: &str =
    "0x35a73cd311a05d46deda634c5ee045db92f811b4e74bca4437fcb5302b7af33";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_json_uses_lowercase_keys() {
        let v = ResourceBounds::zero_fee().to_rpc_json();
        assert!(v.get("l1_gas").is_some());
        assert!(v.get("l2_gas").is_some());
        assert!(v.get("l1_data_gas").is_some());
        assert_eq!(v["l2_gas"]["max_amount"], "0x7000000");
    }

    #[test]
    fn gateway_json_uses_uppercase_keys() {
        let v = ResourceBounds::zero_fee().to_gateway_json();
        assert!(v.get("L1_GAS").is_some());
        assert!(v.get("L2_GAS").is_some());
        assert!(v.get("L1_DATA_GAS").is_some());
    }

    #[test]
    fn from_prices_applies_2x_with_saturation() {
        let b = ResourceBounds::from_prices(10, 20, 30);
        assert_eq!(b.l1_gas.max_price_per_unit, 20);
        assert_eq!(b.l1_data_gas.max_price_per_unit, 40);
        assert_eq!(b.l2_gas.max_price_per_unit, 60);
        // Saturates instead of overflowing.
        let s = ResourceBounds::from_prices(u128::MAX, 0, 0);
        assert_eq!(s.l1_gas.max_price_per_unit, u128::MAX);
    }

    #[test]
    fn wellknown_constants_are_valid_felts() {
        for c in [STRK_TOKEN, OZ_ACCOUNT_CLASS_HASH, BALANCE_OF_SELECTOR] {
            assert!(Felt::from_hex(c).is_ok(), "invalid felt constant: {c}");
        }
    }
}
