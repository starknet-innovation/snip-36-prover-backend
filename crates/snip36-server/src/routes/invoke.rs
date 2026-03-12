use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::rpc::receipt_block_number;
use snip36_core::types::{ResourceBounds, INCREMENT_SELECTOR};
use tracing::info;

use crate::state::AppState;

use super::fund::error_response;

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

    let resource_bounds = ResourceBounds::playground();

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
