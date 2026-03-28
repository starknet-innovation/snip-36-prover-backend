use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use snip36_core::config::Config;
use snip36_core::rpc::StarknetRpc;
use snip36_core::types::Session;
use tokio::sync::{Mutex, RwLock};

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

/// Shared application state, wrapped in `Arc` by Axum.
pub struct AppState {
    pub config: Config,
    pub rpc: StarknetRpc,
    pub sessions: DashMap<String, Session>,
    pub coinflip: RwLock<Option<CoinFlipDeployment>>,
    pub bank: RwLock<Option<BankDeployment>>,
    /// Pending bet commitments keyed by session_id.
    pub commitments: DashMap<String, BetCommitment>,
    /// Mutex to serialize all sncast invocations — prevents nonce races.
    pub sncast_lock: Mutex<()>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let rpc = StarknetRpc::new(&config.rpc_url);

        // Load persisted deployments if available
        let persisted = Self::load_deployments(&config);

        Self {
            coinflip: RwLock::new(persisted.coinflip),
            bank: RwLock::new(persisted.bank),
            config,
            rpc,
            sessions: DashMap::new(),
            commitments: DashMap::new(),
            sncast_lock: Mutex::new(()),
        }
    }

    fn deployments_path(config: &Config) -> std::path::PathBuf {
        config.output_dir.join("deployments.json")
    }

    fn load_deployments(config: &Config) -> PersistedDeployments {
        let path = Self::deployments_path(config);
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                serde_json::from_str(&contents).unwrap_or_default()
            }
            Err(_) => PersistedDeployments::default(),
        }
    }

    /// Persist current deployment state to disk.
    pub async fn save_deployments(&self) {
        let persisted = PersistedDeployments {
            coinflip: self.coinflip.read().await.clone(),
            bank: self.bank.read().await.clone(),
        };
        let path = Self::deployments_path(&self.config);
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Get or create a session for the given ID (returns a clone for reading).
    pub fn get_session(&self, session_id: &str) -> Session {
        self.sessions
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    /// Atomically mutate a session, avoiding lost-update races from clone + write-back.
    pub fn update_session_with(&self, session_id: &str, f: impl FnOnce(&mut Session)) {
        let mut entry = self.sessions.entry(session_id.to_string()).or_default();
        f(entry.value_mut());
    }
}
