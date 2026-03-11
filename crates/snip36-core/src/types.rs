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

impl Default for ResourceBounds {
    fn default() -> Self {
        Self {
            l1_gas: ResourceBound {
                max_amount: 0x0,
                max_price_per_unit: 0xe8d4a51000,
            },
            l2_gas: ResourceBound {
                max_amount: 0x7000000,
                max_price_per_unit: 0x2cb417800,
            },
            l1_data_gas: ResourceBound {
                max_amount: 0x1b0,
                max_price_per_unit: 0x5dc,
            },
        }
    }
}

/// Resource bounds used by the playground web UI (lower l2_gas).
impl ResourceBounds {
    pub fn playground() -> Self {
        Self {
            l1_gas: ResourceBound {
                max_amount: 0x0,
                max_price_per_unit: 0xe8d4a51000,
            },
            l2_gas: ResourceBound {
                max_amount: 0x2000000,
                max_price_per_unit: 0x2cb417800,
            },
            l1_data_gas: ResourceBound {
                max_amount: 0x1b0,
                max_price_per_unit: 0x5dc,
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

    /// Format for the gateway `resource_bounds` field (uppercase keys).
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

/// Parameters for a SNIP-36 proof submission to the gateway.
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
    pub gateway_url: String,
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
}

// Well-known constants

/// STRK token address on integration sepolia.
pub const STRK_TOKEN: &str =
    "0x70a5da4f557b77a9c54546e4bcc900806e28793d8e3eaaa207428d2387249b7";

/// OpenZeppelin Account class hash on integration sepolia.
pub const OZ_ACCOUNT_CLASS_HASH: &str =
    "0x05b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564";

/// Selector for `increment(amount)`.
pub const INCREMENT_SELECTOR: &str =
    "0x7a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d";

/// Selector for `get_counter()`.
pub const GET_COUNTER_SELECTOR: &str =
    "0x3370263ab53343580e77063a719a5865004caff7f367ec136a6cdd34b6786ca";

/// Selector for `balance_of(account)`.
pub const BALANCE_OF_SELECTOR: &str =
    "0x35a73cd311a05d46deda634c5ee045db92f811b4e74bca4437fcb5302b7af33";

/// Default gateway URL for SNIP-36 proof submission.
pub const DEFAULT_GATEWAY_URL: &str = "https://privacy-starknet-integration.starknet.io";
