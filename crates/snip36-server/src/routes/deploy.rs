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
    pub tx_hash: Option<String>,
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

    let address = parse_hex("contract_address", &combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Account deploy failed: {combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &combined);
    let mut block_number = None;

    if let Some(ref hash) = tx_hash {
        info!(tx_hash = %hash, "Deploy account tx submitted");
        match state.rpc.wait_for_tx(hash, 120, 2).await {
            Ok(receipt) => {
                let bn = receipt_block_number(&receipt);
                info!(block_number = ?bn, "Deploy account confirmed");
                block_number = bn;
                if let Some(block) = bn {
                    let _ = state.rpc.wait_for_block_after(block, 120, 2).await;
                }
            }
            Err(e) => {
                return Err(error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string()));
            }
        }
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

#[derive(Deserialize)]
pub struct DeployCounterRequest {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct DeployCounterResponse {
    pub class_hash: String,
    pub contract_address: String,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
}

/// POST /api/deploy-counter
///
/// Declare + deploy the Counter contract (funded by master account).
pub async fn deploy_counter(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeployCounterRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let cwd = state.config.contracts_dir();

    // Declare
    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &state.config.sncast_account(),
            "declare",
            "--contract-name",
            "Counter",
            "--url",
            &state.config.rpc_url,
        ])
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let declare_stdout = String::from_utf8_lossy(&declare_output.stdout);
    let declare_stderr = String::from_utf8_lossy(&declare_output.stderr);
    let declare_combined = format!("{declare_stdout}\n{declare_stderr}");

    // Extract class hash (long hex string, 50+ chars)
    let class_hash = extract_long_hex(&declare_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Declare failed: {declare_combined}"),
        )
    })?;

    // Deploy with random salt
    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));

    let deploy_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &state.config.sncast_account(),
            "deploy",
            "--class-hash",
            &class_hash,
            "--salt",
            &salt,
            "--url",
            &state.config.rpc_url,
        ])
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deploy_stdout = String::from_utf8_lossy(&deploy_output.stdout);
    let deploy_stderr = String::from_utf8_lossy(&deploy_output.stderr);
    let deploy_combined = format!("{deploy_stdout}\n{deploy_stderr}");

    let contract_address = parse_hex("contract_address", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Deploy failed: {deploy_combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &deploy_combined);
    let mut block_number = None;

    if let Some(ref hash) = tx_hash {
        info!(tx_hash = %hash, "Deploy counter tx submitted");
        match state.rpc.wait_for_tx(hash, 120, 2).await {
            Ok(receipt) => {
                let bn = receipt_block_number(&receipt);
                info!(block_number = ?bn, "Deploy counter confirmed");
                block_number = bn;
                if let Some(block) = bn {
                    let _ = state.rpc.wait_for_block_after(block, 120, 2).await;
                }
            }
            Err(e) => {
                return Err(error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string()));
            }
        }
    }

    state.update_session_with(&req.session_id, |session| {
        session.contract_address = Some(contract_address.clone());
        session.class_hash = Some(class_hash.clone());
        session.deploy_block = block_number;
        session.last_reference_block = block_number;
    });

    Ok(Json(DeployCounterResponse {
        class_hash,
        contract_address,
        tx_hash,
        block_number,
    }))
}

/// Extract a long hex string (50+ hex digits) from text — used for class hashes.
fn extract_long_hex(text: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"0x[0-9a-fA-F]{50,}").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}
