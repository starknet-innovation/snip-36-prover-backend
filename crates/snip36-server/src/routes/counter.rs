//! Counter-contract-specific route handlers.
//!
//! These handlers are tied to the bundled Counter example contract and are
//! **not** part of the generic SNIP-36 server API.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::rpc::receipt_block_number;
use snip36_core::selectors::{GET_COUNTER_SELECTOR, INCREMENT_SELECTOR};
use snip36_core::types::{ResourceBound, ResourceBounds};
use tracing::info;

use crate::state::AppState;

use super::fund::{error_response, parse_hex};

// ── Invoke increment ───────────────────────────────────────────

#[derive(Deserialize)]
pub struct InvokeRequest {
    pub session_id: String,
    #[serde(default = "default_amount")]
    pub amount: u64,
    pub signature_r: String,
    pub signature_s: String,
    pub nonce: u64,
}

fn default_amount() -> u64 {
    1
}

#[derive(Serialize)]
pub struct InvokeResponse {
    pub tx_hash: String,
    pub block_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// POST /api/invoke
///
/// Submit a pre-signed increment() invoke transaction via the RPC node.
pub async fn invoke_increment(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InvokeRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session = state.get_session(&req.session_id);

    let contract_address = session
        .contract_address
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No counter contract deployed"))?;

    let account_address = session
        .account_address
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No account deployed"))?;

    // Build multicall calldata: [num_calls, to, selector, calldata_len, ...calldata]
    let calldata = vec![
        "0x1".to_string(),
        contract_address.to_string(),
        INCREMENT_SELECTOR.to_string(),
        "0x1".to_string(),
        format!("{:#x}", req.amount),
    ];

    let resource_bounds = playground_bounds();

    let invoke_tx = serde_json::json!({
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": account_address,
        "calldata": calldata,
        "nonce": format!("{:#x}", req.nonce),
        "resource_bounds": resource_bounds.to_rpc_json(),
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "signature": [req.signature_r, req.signature_s],
    });

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "starknet_addInvokeTransaction",
        "params": { "invoke_transaction": invoke_tx },
        "id": 1,
    });

    let result = state
        .rpc
        .call_raw(payload)
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    if let Some(error) = result.get("error") {
        return Err(error_response(StatusCode::BAD_REQUEST, &error.to_string()));
    }

    let tx_hash = result
        .get("result")
        .and_then(|r| r.get("transaction_hash"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Unexpected RPC response: {result}"),
            )
        })?
        .to_string();

    info!(tx_hash = %tx_hash, "Invoke tx submitted");

    {
        state.update_session_with(&req.session_id, |session| {
            session.last_invoke_tx = Some(tx_hash.clone());
        });
    }

    // Wait for tx inclusion so the prover can reference the correct block
    match state.rpc.wait_for_tx(&tx_hash, 120, 2).await {
        Ok(receipt) => {
            let bn = receipt_block_number(&receipt);
            info!(block_number = ?bn, "Invoke tx confirmed");
            if let Some(block) = bn {
                state.update_session_with(&req.session_id, |session| {
                    session.invoke_block = Some(block);
                });
            }
            Ok(Json(InvokeResponse {
                tx_hash,
                block_number: bn,
                warning: None,
            }))
        }
        Err(_) => Ok(Json(InvokeResponse {
            tx_hash,
            block_number: None,
            warning: Some("Tx submitted but not yet confirmed".to_string()),
        })),
    }
}

// ── Read counter ───────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ReadCounterRequest {
    pub contract_address: String,
}

#[derive(Serialize)]
pub struct ReadCounterResponse {
    pub counter_value: u64,
}

/// POST /api/read-counter
///
/// Call get_counter() on the deployed contract.
pub async fn read_counter(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReadCounterRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let result = state
        .rpc
        .starknet_call(&req.contract_address, GET_COUNTER_SELECTOR, &[])
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let value = result
        .first()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);

    Ok(Json(ReadCounterResponse {
        counter_value: value,
    }))
}

// ── Deploy counter ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DeployCounterRequest {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct DeployCounterResponse {
    pub class_hash: String,
    pub contract_address: String,
    pub tx_hash: String,
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

    if !deploy_output.status.success() {
        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!(
                "Counter deploy failed (exit {}): {deploy_combined}",
                deploy_output.status
            ),
        ));
    }

    let contract_address = parse_hex("contract_address", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Deploy failed: {deploy_combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Deploy succeeded but no transaction_hash in output: {deploy_combined}"),
        )
    })?;

    info!(tx_hash = %tx_hash, "Deploy counter tx submitted");
    let receipt = state
        .rpc
        .wait_for_tx(&tx_hash, 120, 2)
        .await
        .map_err(|e| error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string()))?;
    let block_number = receipt_block_number(&receipt);
    info!(block_number = ?block_number, "Deploy counter confirmed");
    if let Some(block) = block_number {
        let _ = state.rpc.wait_for_block_after(block, 120, 2).await;
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

// ── Shared helpers ─────────────────────────────────────────────

/// Resource bounds tuned for the playground web UI (lower l2_gas).
pub fn playground_bounds() -> ResourceBounds {
    const SEPOLIA_GAS_PRICE_CEIL: u128 = 0x38d7ea4c68000;
    ResourceBounds {
        l1_gas: ResourceBound {
            max_amount: 0x0,
            max_price_per_unit: SEPOLIA_GAS_PRICE_CEIL,
        },
        l2_gas: ResourceBound {
            max_amount: 0x2000000,
            max_price_per_unit: 0x2cb417800,
        },
        l1_data_gas: ResourceBound {
            max_amount: 0x1b0,
            max_price_per_unit: SEPOLIA_GAS_PRICE_CEIL,
        },
    }
}
