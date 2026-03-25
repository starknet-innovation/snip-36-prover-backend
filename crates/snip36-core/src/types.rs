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
                max_amount: 0x10000,
                max_price_per_unit: 0x3a3529440000, // ~64T — covers sepolia gas spikes
            },
            l2_gas: ResourceBound {
                max_amount: 0x7000000,
                max_price_per_unit: 0x1dcd65000,
            },
            l1_data_gas: ResourceBound {
                max_amount: 0x1b0,
                max_price_per_unit: 0x100000, // ~1M — covers sepolia data gas spikes
            },
        }
    }
}

impl ResourceBounds {
    /// Resource bounds for virtual OS proving: enough gas for execution, zero price (no fee).
    pub fn zero_fee() -> Self {
        Self {
            l1_gas: ResourceBound { max_amount: 0, max_price_per_unit: 0 },
            l2_gas: ResourceBound { max_amount: 0x7000000, max_price_per_unit: 0 },
            l1_data_gas: ResourceBound { max_amount: 0x1b0, max_price_per_unit: 0 },
        }
    }

    /// Resource bounds used by the playground web UI (lower l2_gas).
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
    /// Block number where the counter contract was deployed.
    pub deploy_block: Option<u64>,
    /// Reference block for the next prove (updated after each proven tx lands).
    pub last_reference_block: Option<u64>,
}

// Well-known constants

/// STRK token address on sepolia.
pub const STRK_TOKEN: &str =
    "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

/// OpenZeppelin Account class hash on sepolia.
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

/// Selector for `send_message(to_address, payload)`.
pub const SEND_MESSAGE_SELECTOR: &str =
    "0x12ead94ae9d3f9d2bdb6b847cf255f1f398193a1f88884a0ae8e18f24a037b6";

/// Selector for `play(seed, player, bet)` (CoinFlip contract).
pub const PLAY_SELECTOR: &str =
    "0x21c4a0db2b08b026c4e31bf76d5dd9b92aa54c0978df57474355786073775e8";

