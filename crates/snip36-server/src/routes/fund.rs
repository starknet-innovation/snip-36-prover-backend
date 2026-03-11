use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::rpc::receipt_block_number;
use snip36_core::types::STRK_TOKEN;
use tracing::info;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct FundRequest {
    pub account_address: String,
}

#[derive(Serialize)]
pub struct FundResponse {
    pub tx_hash: String,
    pub amount: String,
    pub block_number: Option<u64>,
}

/// POST /api/fund
///
/// Transfer 10 STRK from master account to target address using sncast.
pub async fn fund_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<FundRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let amount_wei: u128 = 10u128.pow(16); // 0.01 STRK
    let amount_low = format!("{:#x}", amount_wei);
    let amount_high = "0x0";

    let calldata = format!("{} {} {}", req.account_address, amount_low, amount_high);

    let output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            "ci-health-check",
            "invoke",
            "--contract-address",
            STRK_TOKEN,
            "--function",
            "transfer",
            "--calldata",
            &calldata,
            "--url",
            &state.config.rpc_url,
        ])
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    let tx_hash = parse_hex("transaction_hash", &combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Transfer failed: {combined}"),
        )
    })?;

    info!(tx_hash = %tx_hash, "Fund tx submitted");

    let receipt = state
        .rpc
        .wait_for_tx(&tx_hash, 120, 2)
        .await
        .map_err(|e| error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string()))?;

    let bn = receipt_block_number(&receipt);
    info!(block_number = ?bn, "Fund tx confirmed");

    if let Some(block) = bn {
        let _ = state.rpc.wait_for_block_after(block, 120, 2).await;
    }

    Ok(Json(FundResponse {
        tx_hash,
        amount: "0.01 STRK".to_string(),
        block_number: bn,
    }))
}

/// Parse a hex value after a given key from sncast output.
/// Matches flexibly: underscores and spaces are treated as equivalent.
pub fn parse_hex(key: &str, text: &str) -> Option<String> {
    let pattern = key.replace('_', "[_ ]");
    let re = regex_lite::Regex::new(&format!("(?i){pattern}")).ok()?;
    let hex_re = regex_lite::Regex::new(r"0x[0-9a-fA-F]+").ok()?;
    for line in text.lines() {
        if re.is_match(line) {
            if let Some(m) = hex_re.find(line) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

pub fn error_response(status: StatusCode, detail: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "detail": detail })))
}
