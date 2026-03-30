pub mod deploy;
pub mod fund;
pub mod prove;
pub mod prove_block;
pub mod read;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Build the generic SNIP-36 API router.
///
/// Application-specific routes (Counter, CoinFlip) are added by the binary
/// that composes the full server (see `apps/playground`).
pub fn generic_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(read::health))
        .route("/api/fund", post(fund::fund_account))
        .route("/api/deploy-account", post(deploy::deploy_account))
        .route("/api/prove/{session_id}", get(prove::prove_transaction))
        .route("/api/nonce/{address}", get(read::get_nonce))
}
