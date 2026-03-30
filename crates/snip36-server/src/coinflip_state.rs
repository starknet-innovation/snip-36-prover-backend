//! CoinFlip-specific state types.
//!
//! Extracted from the generic AppState so the boundary between core SNIP-36
//! infrastructure and the CoinFlip example is clear.

use serde::{Deserialize, Serialize};

/// Deployed CoinFlip contract info (shared across all sessions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinFlipDeployment {
    pub contract_address: String,
    pub class_hash: String,
    pub deploy_block: u64,
}

/// Deployed CoinFlipBank contract info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankDeployment {
    pub contract_address: String,
    pub class_hash: String,
    pub deploy_block: u64,
}

/// Persisted deployment state (saved to output/deployments.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedDeployments {
    pub coinflip: Option<CoinFlipDeployment>,
    pub bank: Option<BankDeployment>,
}

/// A committed bet waiting to be revealed.
#[derive(Debug, Clone)]
pub struct BetCommitment {
    /// pedersen(bet, nonce) — computed by the player
    pub commitment: String,
    /// Block number locked at commit time (used as seed)
    pub seed_block: u64,
    /// Player address
    pub player: String,
    /// Bet amount in wei (hex string), set after deposit-info
    pub bet_amount: Option<String>,
    /// Session ID as felt252 hex (for on-chain contract)
    pub session_felt: String,
}
