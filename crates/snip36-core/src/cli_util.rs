//! CLI helper utilities for parsing subprocess output.
//!
//! Available when the `cli` feature is enabled.

use std::process::Output;
use std::time::Duration;

use tokio::process::Command;
use tracing::info;

use crate::types::ResourceBounds;

const SNCAST_RETRY_ATTEMPTS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SncastFailureKind {
    TransientProvider,
    Rejected,
    UnexpectedOutput,
}

impl SncastFailureKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::TransientProvider => "transient RPC/provider response",
            Self::Rejected => "contract transaction rejected",
            Self::UnexpectedOutput => "unexpected sncast output",
        }
    }
}

/// Build explicit sncast resource-bound arguments from live gas prices.
///
/// This avoids sncast fee estimation while still tracking Sepolia gas price
/// movement instead of relying on stale hardcoded price caps.
pub fn sncast_resource_bound_args(bounds: &ResourceBounds) -> Vec<String> {
    vec![
        "--l1-gas".to_string(),
        bounds.l1_gas.max_amount.to_string(),
        "--l1-gas-price".to_string(),
        bounds.l1_gas.max_price_per_unit.to_string(),
        "--l2-gas".to_string(),
        bounds.l2_gas.max_amount.to_string(),
        "--l2-gas-price".to_string(),
        bounds.l2_gas.max_price_per_unit.to_string(),
        "--l1-data-gas".to_string(),
        bounds.l1_data_gas.max_amount.to_string(),
        "--l1-data-gas-price".to_string(),
        bounds.l1_data_gas.max_price_per_unit.to_string(),
    ]
}

pub fn classify_sncast_failure(status_success: bool, text: &str) -> SncastFailureKind {
    if is_transient_sncast_failure(status_success, text) {
        SncastFailureKind::TransientProvider
    } else if status_success {
        SncastFailureKind::UnexpectedOutput
    } else {
        SncastFailureKind::Rejected
    }
}

pub async fn run_sncast_with_retries<F>(
    operation: &str,
    mut build_command: F,
) -> std::io::Result<Output>
where
    F: FnMut() -> Command,
{
    for attempt in 1..=SNCAST_RETRY_ATTEMPTS {
        let output = build_command().output().await?;
        let text = format_cmd_output(&output);

        if output.status.success()
            || !matches!(
                classify_sncast_failure(output.status.success(), &text),
                SncastFailureKind::TransientProvider
            )
            || attempt == SNCAST_RETRY_ATTEMPTS
        {
            return Ok(output);
        }

        let delay = Duration::from_secs(5 * attempt as u64);
        info!(
            "  {operation} attempt {attempt}/{SNCAST_RETRY_ATTEMPTS} hit a transient RPC/provider response; retrying in {}s...",
            delay.as_secs()
        );
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            info!(
                "  transient output: {}",
                trimmed.get(..500).unwrap_or(trimmed)
            );
        }
        tokio::time::sleep(delay).await;
    }

    unreachable!("sncast retry loop always returns from inside the loop")
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

/// Format subprocess output with exit status for diagnostics.
pub fn format_cmd_output_with_status(output: &Output) -> String {
    format!("status: {}\n{}", output.status, format_cmd_output(output))
}

fn is_transient_sncast_failure(status_success: bool, text: &str) -> bool {
    if status_success {
        return false;
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    [
        "expected value at line 1 column 1",
        "unknown rpc error",
        "non-json",
        "empty response",
        "bad gateway",
        "service unavailable",
        "gateway timeout",
        "too many requests",
        "rate limit",
        "rate-limit",
        "http 429",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
        "timed out",
        "timeout",
        "connection reset",
        "connection refused",
        "error trying to connect",
        "temporarily unavailable",
        "<html",
        "<!doctype html",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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
    fn resource_bound_args_use_live_prices() {
        let bounds = ResourceBounds::from_prices(11, 13, 17);
        assert_eq!(
            sncast_resource_bound_args(&bounds),
            vec![
                "--l1-gas",
                "65536",
                "--l1-gas-price",
                "22",
                "--l2-gas",
                "117440512",
                "--l2-gas-price",
                "34",
                "--l1-data-gas",
                "432",
                "--l1-data-gas-price",
                "26",
            ]
        );
    }

    #[test]
    fn classify_sncast_transient_provider_failures() {
        assert_eq!(
            classify_sncast_failure(
                false,
                "stderr: Error: Unknown RPC error: expected value at line 1 column 1"
            ),
            SncastFailureKind::TransientProvider
        );
        assert_eq!(
            classify_sncast_failure(false, "Error: Contract failed the validation = nope"),
            SncastFailureKind::Rejected
        );
        assert_eq!(
            classify_sncast_failure(true, "Success: no address here"),
            SncastFailureKind::UnexpectedOutput
        );
    }
}
