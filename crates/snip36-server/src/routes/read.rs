use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::types::GET_COUNTER_SELECTOR;

use crate::state::AppState;

use super::fund::error_response;

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

#[derive(Serialize)]
pub struct NonceResponse {
    pub nonce: u64,
    pub nonce_hex: String,
}

/// GET /api/nonce/{address}
///
/// Fetch the current nonce for an account.
pub async fn get_nonce(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let nonce = state
        .rpc
        .get_nonce(&address)
        .await
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, &e.to_string()))?;

    Ok(Json(NonceResponse {
        nonce,
        nonce_hex: format!("{:#x}", nonce),
    }))
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub rpc_url: String,
}

/// GET /api/health
pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        rpc_url: state.config.rpc_url.clone(),
    })
}
