use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::rpc::receipt_block_number;
use snip36_core::types::OZ_ACCOUNT_CLASS_HASH;
use tracing::info;

use crate::state::AppState;

use super::fund::{error_response, parse_hex};

#[derive(Deserialize)]
pub struct DeployAccountRequest {
    pub session_id: String,
    pub public_key: String,
    pub account_address: String,
}

#[derive(Serialize)]
pub struct DeployAccountResponse {
    pub account_address: String,
    pub tx_hash: String,
    pub block_number: Option<u64>,
}

/// POST /api/deploy-account
///
/// Deploy an OpenZeppelin account contract for the generated key.
pub async fn deploy_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeployAccountRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &state.config.sncast_account(),
            "deploy",
            "--class-hash",
            OZ_ACCOUNT_CLASS_HASH,
            "--constructor-calldata",
            &req.public_key,
            "--salt",
            &req.public_key,
            "--url",
            &state.config.rpc_url,
        ])
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if !output.status.success() {
        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Account deploy failed (exit {}): {combined}", output.status),
        ));
    }

    let address = parse_hex("contract_address", &combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Account deploy failed: {combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Deploy succeeded but no transaction_hash in output: {combined}"),
        )
    })?;

    info!(tx_hash = %tx_hash, "Deploy account tx submitted");
    let receipt = state
        .rpc
        .wait_for_tx(&tx_hash, 120, 2)
        .await
        .map_err(|e| error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string()))?;
    let block_number = receipt_block_number(&receipt);
    info!(block_number = ?block_number, "Deploy account confirmed");
    if let Some(block) = block_number {
        let _ = state.rpc.wait_for_block_after(block, 120, 2).await;
    }

    // Store the actual deployed address, not the client-provided one
    if address != req.account_address {
        info!(
            expected = %req.account_address,
            actual = %address,
            "Deployed address differs from requested"
        );
    }
    state.update_session_with(&req.session_id, |session| {
        session.account_address = Some(address.clone());
        session.account_deployed = true;
    });

    Ok(Json(DeployAccountResponse {
        account_address: address,
        tx_hash,
        block_number,
    }))
}
