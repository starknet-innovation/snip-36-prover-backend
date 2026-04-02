//! Generic SNIP-36 server library.
//!
//! Provides reusable Axum route handlers for the core proving pipeline
//! (deploy account, fund, prove, health, nonce) and the shared [`AppState`].
//! Application-specific routes (Counter, CoinFlip) live in separate app crates.

pub mod routes;
pub mod state;

pub use state::AppState;
