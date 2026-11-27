use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

/// Parse a Starknet field element and return its canonical hexadecimal encoding.
///
/// This is intended for values that cross a trust boundary before being used as
/// command-line arguments or persisted identifiers. Reformatting from `Felt`
/// ensures the result contains only a `0x` prefix and hexadecimal digits, and
/// is within the Starknet field range.
pub fn canonical_felt_hex(value: &str) -> Result<String, String> {
    let felt = Felt::from_hex(value).map_err(|e| format!("invalid Starknet field element: {e}"))?;
    Ok(format!("{felt:#x}"))
}

/// Default `l2_gas.max_amount` of the virtual (zero-fee) transaction.
pub const DEFAULT_VIRTUAL_L2_GAS: u64 = 0x7000000;

/// Largest useful `l2_gas.max_amount` for the virtual transaction.
///
/// The virtual OS starts the transaction with `l2_gas.max_amount` Sierra gas
/// and caps each entry point with the Starknet versioned constants:
/// `execute_max_sierra_gas` (1,110,000,000) for `__execute__` and
/// `validate_max_sierra_gas` (100,000,000) for `__validate__`
/// (Starknet 0.14.3 / 0.14.4). Anything above the execute cap buys nothing.
pub const MAX_VIRTUAL_L2_GAS: u64 = 1_110_000_000;

/// Environment variable overriding the virtual transaction's `l2_gas.max_amount`.
pub const VIRTUAL_L2_GAS_ENV: &str = "SNIP36_VIRTUAL_L2_GAS";

/// Invalid L2 gas budget for the virtual transaction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VirtualL2GasError {
    #[error("invalid virtual L2 gas amount '{0}': expected a decimal or 0x-prefixed hex integer")]
    Invalid(String),
    #[error(
        "virtual L2 gas amount {0} exceeds the protocol execute cap of {MAX_VIRTUAL_L2_GAS} \
         (execute_max_sierra_gas)"
    )]
    AboveCap(u64),
}

/// Parse and validate a virtual-transaction L2 gas amount (decimal or `0x` hex).
pub fn parse_virtual_l2_gas(value: &str) -> Result<u64, VirtualL2GasError> {
    let trimmed = value.trim();
    let parsed = match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => trimmed.parse::<u64>(),
    };
    let amount = parsed.map_err(|_| VirtualL2GasError::Invalid(value.to_string()))?;
    check_virtual_l2_gas(amount)
}

fn check_virtual_l2_gas(amount: u64) -> Result<u64, VirtualL2GasError> {
    if amount > MAX_VIRTUAL_L2_GAS {
        return Err(VirtualL2GasError::AboveCap(amount));
    }
    Ok(amount)
}

/// Resolve the virtual transaction's L2 gas budget: explicit value (CLI flag),
/// else the environment value, else [`DEFAULT_VIRTUAL_L2_GAS`].
///
/// An empty environment value counts as unset.
pub fn resolve_virtual_l2_gas(
    explicit: Option<u64>,
    env_value: Option<&str>,
) -> Result<u64, VirtualL2GasError> {
    if let Some(amount) = explicit {
        return check_virtual_l2_gas(amount);
    }
    match env_value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => parse_virtual_l2_gas(value),
        None => Ok(DEFAULT_VIRTUAL_L2_GAS),
    }
}

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
    ///
    /// Uses [`DEFAULT_VIRTUAL_L2_GAS`]; see [`Self::zero_fee_with_l2_gas`] to raise it.
    pub fn zero_fee() -> Self {
        Self::zero_fee_unchecked(DEFAULT_VIRTUAL_L2_GAS)
    }

    /// Zero-fee bounds for the virtual transaction with a custom `l2_gas.max_amount`.
    ///
    /// Rejects amounts above [`MAX_VIRTUAL_L2_GAS`] instead of clamping them:
    /// the bounds are part of the signed transaction hash.
    pub fn zero_fee_with_l2_gas(l2_gas_amount: u64) -> Result<Self, VirtualL2GasError> {
        Ok(Self::zero_fee_unchecked(check_virtual_l2_gas(
            l2_gas_amount,
        )?))
    }

    /// Zero-fee bounds for the virtual transaction, with the L2 gas budget taken
    /// from `explicit` (CLI flag), else `SNIP36_VIRTUAL_L2_GAS`, else the default.
    pub fn zero_fee_from_env(explicit: Option<u64>) -> Result<Self, VirtualL2GasError> {
        let env_value = std::env::var(VIRTUAL_L2_GAS_ENV).ok();
        Self::zero_fee_with_l2_gas(resolve_virtual_l2_gas(explicit, env_value.as_deref())?)
    }

    fn zero_fee_unchecked(l2_gas_amount: u64) -> Self {
        Self {
            l1_gas: ResourceBound {
                max_amount: 0,
                max_price_per_unit: 0,
            },
            l2_gas: ResourceBound {
                max_amount: l2_gas_amount,
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
    fn zero_fee_default_l2_gas_is_unchanged() {
        let b = ResourceBounds::zero_fee();
        assert_eq!(DEFAULT_VIRTUAL_L2_GAS, 117_440_512);
        assert_eq!(b.l2_gas.max_amount, 0x7000000);
        assert_eq!(b.l2_gas.max_price_per_unit, 0);
        assert_eq!(b.l1_gas.max_amount, 0);
        assert_eq!(b.l1_data_gas.max_amount, 0x1b0);
        // No override anywhere resolves to the same bounds.
        let resolved = resolve_virtual_l2_gas(None, None).unwrap();
        assert_eq!(
            ResourceBounds::zero_fee_with_l2_gas(resolved)
                .unwrap()
                .to_rpc_json(),
            b.to_rpc_json()
        );
    }

    #[test]
    fn zero_fee_with_l2_gas_applies_override_only_to_l2_amount() {
        let b = ResourceBounds::zero_fee_with_l2_gas(MAX_VIRTUAL_L2_GAS).unwrap();
        let v = b.to_rpc_json();
        assert_eq!(v["l2_gas"]["max_amount"], "0x42294180");
        assert_eq!(v["l2_gas"]["max_price_per_unit"], "0x0");
        assert_eq!(
            v["l1_gas"],
            ResourceBounds::zero_fee().to_rpc_json()["l1_gas"]
        );
        assert_eq!(
            v["l1_data_gas"],
            ResourceBounds::zero_fee().to_rpc_json()["l1_data_gas"]
        );
    }

    #[test]
    fn virtual_l2_gas_above_execute_cap_is_rejected() {
        assert_eq!(
            ResourceBounds::zero_fee_with_l2_gas(MAX_VIRTUAL_L2_GAS + 1).unwrap_err(),
            VirtualL2GasError::AboveCap(MAX_VIRTUAL_L2_GAS + 1)
        );
        assert!(parse_virtual_l2_gas("0x42294181").is_err());
        assert!(resolve_virtual_l2_gas(Some(u64::MAX), None).is_err());
        assert!(resolve_virtual_l2_gas(None, Some("2000000000")).is_err());
        let msg = VirtualL2GasError::AboveCap(MAX_VIRTUAL_L2_GAS + 1).to_string();
        assert!(
            msg.contains("1110000001") && msg.contains("1110000000"),
            "{msg}"
        );
    }

    #[test]
    fn virtual_l2_gas_parses_decimal_and_hex() {
        assert_eq!(parse_virtual_l2_gas("1110000000"), Ok(MAX_VIRTUAL_L2_GAS));
        assert_eq!(parse_virtual_l2_gas("0x42294180"), Ok(MAX_VIRTUAL_L2_GAS));
        assert_eq!(
            parse_virtual_l2_gas(" 0X7000000 "),
            Ok(DEFAULT_VIRTUAL_L2_GAS)
        );
        for bad in ["", "abc", "-1", "0x", "1e9"] {
            assert!(
                matches!(
                    parse_virtual_l2_gas(bad),
                    Err(VirtualL2GasError::Invalid(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn virtual_l2_gas_resolution_prefers_explicit_then_env_then_default() {
        assert_eq!(resolve_virtual_l2_gas(Some(5), Some("7")), Ok(5));
        assert_eq!(resolve_virtual_l2_gas(None, Some("0x7")), Ok(7));
        assert_eq!(
            resolve_virtual_l2_gas(None, Some("  ")),
            Ok(DEFAULT_VIRTUAL_L2_GAS)
        );
        assert_eq!(
            resolve_virtual_l2_gas(None, None),
            Ok(DEFAULT_VIRTUAL_L2_GAS)
        );
        assert!(resolve_virtual_l2_gas(None, Some("nope")).is_err());
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
        // Real (fee-paying) transaction bounds are independent of the virtual override.
        assert_eq!(b.l2_gas.max_amount, 0x7000000);
    }

    #[test]
    fn wellknown_constants_are_valid_felts() {
        for c in [STRK_TOKEN, OZ_ACCOUNT_CLASS_HASH, BALANCE_OF_SELECTOR] {
            assert!(Felt::from_hex(c).is_ok(), "invalid felt constant: {c}");
        }
    }

    #[test]
    fn canonical_felt_hex_reformats_and_validates_input() {
        assert_eq!(canonical_felt_hex("0x000001").as_deref(), Ok("0x1"));
        assert!(canonical_felt_hex("not-a-felt").is_err());
        assert!(canonical_felt_hex(
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        )
        .is_err());
    }
}
