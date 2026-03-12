use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::signing::{felt_from_hex, sign_and_build_payload};
use snip36_core::types::{ResourceBounds, SubmitParams, INCREMENT_SELECTOR};
use starknet_types_core::felt::Felt;
use tracing::info;

use crate::state::AppState;

use super::fund::error_response;

#[derive(Deserialize)]
pub struct SubmitProofRequest {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct SubmitProofResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    pub output: String,
}

/// POST /api/submit-proof
///
/// Sign and submit the proof-bearing transaction to the privacy gateway.
pub async fn submit_proof(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitProofRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session = state.get_session(&req.session_id);

    let proof_file = session
        .proof_file
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No proof file available"))?;

    let proof_path = Path::new(proof_file);
    if !proof_path.exists() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No proof file available",
        ));
    }

    let contract_address = session
        .contract_address
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No counter contract in session"))?;

    // Read proof (base64 string)
    let proof_base64 = tokio::fs::read_to_string(proof_file)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to read proof file: {e}"),
            )
        })?
        .trim()
        .to_string();

    // Read proof_facts
    let proof_facts_file = proof_file.replace(".proof", ".proof_facts");
    let proof_facts_json = tokio::fs::read_to_string(&proof_facts_file)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to read proof_facts file: {e}"),
            )
        })?;

    let proof_facts_hex: Vec<String> = serde_json::from_str(&proof_facts_json).map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Invalid proof_facts JSON: {e}"),
        )
    })?;

    let proof_facts: Vec<Felt> = proof_facts_hex
        .iter()
        .map(|h| felt_from_hex(h))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    // Build calldata for increment(1): [num_calls, to, selector, calldata_len, amount]
    let calldata = vec![
        Felt::ONE,
        felt_from_hex(contract_address)
            .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?,
        felt_from_hex(INCREMENT_SELECTOR)
            .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?,
        Felt::ONE,
        Felt::ONE,
    ];

    // Sign and submit as the configured master account (whose private key we have)
    let sender_address = felt_from_hex(&state.config.account_address)
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let private_key = felt_from_hex(&state.config.private_key)
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let chain_id = state.config.chain_id_felt().map_err(|e| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
    })?;

    // Get nonce for the master account
    let nonce = state
        .rpc
        .get_nonce(&state.config.account_address)
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let params = SubmitParams {
        sender_address,
        private_key,
        calldata,
        proof_base64,
        proof_facts,
        nonce: Felt::from(nonce),
        chain_id,
        resource_bounds: ResourceBounds::playground(),
        gateway_url: state.config.gateway_url.clone(),
    };

    let (tx_hash, payload) = sign_and_build_payload(&params)
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let tx_hash_hex = format!("{:#x}", tx_hash);
    info!(tx_hash = %tx_hash_hex, "Submitting proof to gateway");

    // Submit to gateway via HTTP POST
    let gateway_url = format!(
        "{}/gateway/add_transaction",
        state.config.gateway_url.trim_end_matches('/')
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&gateway_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Gateway request failed: {e}"),
            )
        })?;

    let status = resp.status();
    let resp_text = resp.text().await.map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to read gateway response: {e}"),
        )
    })?;

    info!(status = %status, response = %resp_text, "Gateway response");

    if !status.is_success() {
        return Err(error_response(
            StatusCode::BAD_GATEWAY,
            &format!("Gateway rejected transaction (HTTP {status}): {resp_text}"),
        ));
    }

    // Verify gateway accepted the transaction
    if let Ok(resp_json) = serde_json::from_str::<serde_json::Value>(&resp_text) {
        if let Some(code) = resp_json.get("code").and_then(|c| c.as_str()) {
            if code != "TRANSACTION_RECEIVED" {
                return Err(error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("Gateway rejected transaction (code={code}): {resp_text}"),
                ));
            }
        }
    }

    Ok(Json(SubmitProofResponse {
        tx_hash: Some(tx_hash_hex),
        output: resp_text,
    }))
}
