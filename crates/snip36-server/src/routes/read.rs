use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

use super::fund::error_response;

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
