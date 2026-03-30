pub mod coinflip;
pub mod counter;
pub mod deploy;
pub mod fund;
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
        // ── Generic SNIP-36 routes ──────────────────────────
        .route("/api/health", get(read::health))
        .route("/api/fund", post(fund::fund_account))
        .route("/api/deploy-account", post(deploy::deploy_account))
        .route("/api/prove/{session_id}", get(prove::prove_transaction))
        .route("/api/submit-proof", post(submit::submit_proof))
        .route("/api/nonce/{address}", get(read::get_nonce))
        // ── Counter example routes ──────────────────────────
        .route("/api/deploy-counter", post(counter::deploy_counter))
        .route("/api/invoke", post(counter::invoke_increment))
        .route("/api/read-counter", post(counter::read_counter))
        .route(
            "/api/prove-block/{session_id}",
            get(prove_block::prove_block),
        )
        // ── CoinFlip routes ─────────────────────────────────
        .route("/api/coinflip/status", get(coinflip::coinflip_status))
        .route("/api/coinflip/deploy", post(coinflip::deploy_coinflip))
        .route("/api/coinflip/commit", post(coinflip::commit_bet))
        .route(
            "/api/coinflip/play/{session_id}",
            get(coinflip::play_coinflip),
        )
        // CoinFlipBank routes
        .route("/api/coinflip/bank/status", get(coinflip::bank_status))
        .route("/api/coinflip/bank/deploy", post(coinflip::deploy_bank))
        .route("/api/coinflip/deposit-info", post(coinflip::deposit_info))
        .route(
            "/api/coinflip/confirm-deposit",
            post(coinflip::confirm_deposit),
        )
        .route(
            "/api/coinflip/balance/{address}",
            get(coinflip::player_balance),
        )
        .route(
            "/api/coinflip/winnings/{address}",
            get(coinflip::player_winnings),
        )
}
