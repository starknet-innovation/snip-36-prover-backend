//! CLI helper utilities for parsing subprocess output.
//!
//! Available when the `cli` feature is enabled.

use std::process::Output;

const SNCAST_L1_GAS: u64 = 0;
const SNCAST_L2_GAS: u64 = 33_554_432;
const SNCAST_L1_DATA_GAS: u64 = 432;
const SNCAST_GAS_PRICE_MULTIPLIER: u128 = 2;

/// Build resource-bound arguments for lightweight sncast E2E declare/deploy calls.
///
/// The gas amounts stay intentionally small for declare/deploy, while prices
/// come from the latest block header with headroom for short-term fee movement.
pub fn sncast_resource_bound_args(
    l1_gas_price: u128,
    l1_data_gas_price: u128,
    l2_gas_price: u128,
) -> Vec<String> {
    vec![
        "--l1-gas".to_string(),
        SNCAST_L1_GAS.to_string(),
        "--l1-gas-price".to_string(),
        l1_gas_price
            .saturating_mul(SNCAST_GAS_PRICE_MULTIPLIER)
            .to_string(),
        "--l2-gas".to_string(),
        SNCAST_L2_GAS.to_string(),
        "--l2-gas-price".to_string(),
        l2_gas_price
            .saturating_mul(SNCAST_GAS_PRICE_MULTIPLIER)
            .to_string(),
        "--l1-data-gas".to_string(),
        SNCAST_L1_DATA_GAS.to_string(),
        "--l1-data-gas-price".to_string(),
        l1_data_gas_price
            .saturating_mul(SNCAST_GAS_PRICE_MULTIPLIER)
            .to_string(),
    ]
}

/// Extract a hex value (0x...) from text after a flexible key match.
///
/// Handles sncast output where "class_hash" may appear as "Class Hash",
/// "class hash", "class_hash", etc.
pub fn parse_hex_from_output(key: &str, text: &str) -> Option<String> {
    let pattern = key.replace('_', "[_ ]");
    let re = regex_lite::Regex::new(&format!("(?i){pattern}")).ok()?;
    for line in text.lines() {
        if re.is_match(line) {
            if let Some(m) = regex_lite::Regex::new(r"0x[0-9a-fA-F]+").ok()?.find(line) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

/// Extract a long hex value (50+ hex chars) from text, useful for class hashes.
pub fn parse_long_hex(text: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"0x[0-9a-fA-F]{50,}").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

/// Format a subprocess output for error reporting.
pub fn format_cmd_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        format!("stdout: {stdout}\nstderr: {stderr}")
    } else {
        stdout.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_matches_flexible_key() {
        // sncast prints "Class Hash: 0x..."; the key "class_hash" should match
        // regardless of case or underscore-vs-space.
        let text = "Command: declare\nClass Hash: 0x05b4b537eaa2399e\nok";
        assert_eq!(
            parse_hex_from_output("class_hash", text).as_deref(),
            Some("0x05b4b537eaa2399e")
        );
        assert_eq!(parse_hex_from_output("class_hash", "no match here"), None);
    }

    #[test]
    fn parse_long_hex_needs_50_plus_chars() {
        assert_eq!(parse_long_hex("addr 0x1234"), None);
        let long = format!("class_hash: 0x{}", "a".repeat(60));
        assert_eq!(parse_long_hex(&long).unwrap().len(), 62); // "0x" + 60 hex chars
    }

    #[test]
    fn sncast_resource_bound_args_use_live_prices_with_headroom() {
        assert_eq!(
            sncast_resource_bound_args(10, 20, 30),
            vec![
                "--l1-gas",
                "0",
                "--l1-gas-price",
                "20",
                "--l2-gas",
                "33554432",
                "--l2-gas-price",
                "60",
                "--l1-data-gas",
                "432",
                "--l1-data-gas-price",
                "40",
            ]
        );
    }
}
