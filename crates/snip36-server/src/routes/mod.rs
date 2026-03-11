pub mod deploy;
pub mod fund;
pub mod invoke;
pub mod prove;
pub mod read;
pub mod submit;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Build the full API router under `/api/`.
pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(read::health))
        .route("/api/fund", post(fund::fund_account))
        .route("/api/deploy-account", post(deploy::deploy_account))
        .route("/api/deploy-counter", post(deploy::deploy_counter))
        .route("/api/invoke", post(invoke::invoke_increment))
        .route("/api/prove/{session_id}", get(prove::prove_transaction))
        .route("/api/submit-proof", post(submit::submit_proof))
        .route("/api/read-counter", post(read::read_counter))
        .route("/api/nonce/{address}", get(read::get_nonce))
}
