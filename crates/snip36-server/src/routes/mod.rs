pub mod coinflip;
pub mod deploy;
pub mod fund;
pub mod invoke;
pub mod prove;
pub mod prove_block;
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
        .route(
            "/api/prove-block/{session_id}",
            get(prove_block::prove_block),
        )
        .route("/api/submit-proof", post(submit::submit_proof))
        .route("/api/read-counter", post(read::read_counter))
        .route("/api/nonce/{address}", get(read::get_nonce))
        // CoinFlip routes
        .route("/api/coinflip/status", get(coinflip::coinflip_status))
        .route("/api/coinflip/deploy", post(coinflip::deploy_coinflip))
        .route("/api/coinflip/commit", post(coinflip::commit_bet))
        .route(
            "/api/coinflip/play/{session_id}",
            get(coinflip::play_coinflip),
        )
}
