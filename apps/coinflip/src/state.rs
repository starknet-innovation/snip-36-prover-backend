//! CoinFlip-specific state types.
//!
//! Extracted from the generic AppState so the boundary between core SNIP-36
//! infrastructure and the CoinFlip example is clear.

use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use snip36_server::AppState;

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

/// Composite state for CoinFlip routes — wraps generic AppState + CoinFlip-specific state.
pub struct CoinFlipAppState {
    pub app: Arc<AppState>,
    pub coinflip: RwLock<Option<CoinFlipDeployment>>,
    pub bank: RwLock<Option<BankDeployment>>,
    pub commitments: DashMap<String, BetCommitment>,
}

impl CoinFlipAppState {
    pub fn new(app: Arc<AppState>) -> Self {
        let persisted = Self::load_deployments(&app.config);
        Self {
            coinflip: RwLock::new(persisted.coinflip),
            bank: RwLock::new(persisted.bank),
            commitments: DashMap::new(),
            app,
        }
    }

    fn deployments_path(config: &snip36_core::Config) -> std::path::PathBuf {
        config.output_dir.join("deployments.json")
    }

    fn load_deployments(config: &snip36_core::Config) -> PersistedDeployments {
        let path = Self::deployments_path(config);
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => PersistedDeployments::default(),
        }
    }

    pub async fn save_deployments(&self) {
        let persisted = PersistedDeployments {
            coinflip: self.coinflip.read().await.clone(),
            bank: self.bank.read().await.clone(),
        };
        let path = Self::deployments_path(&self.app.config);
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(&path, json);
        }
    }
}
