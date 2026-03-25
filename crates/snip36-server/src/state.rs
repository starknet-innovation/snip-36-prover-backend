use dashmap::DashMap;
use snip36_core::config::Config;
use snip36_core::rpc::StarknetRpc;
use snip36_core::types::Session;
use tokio::sync::RwLock;

/// Deployed CoinFlip contract info (shared across all sessions).
#[derive(Debug, Clone)]
pub struct CoinFlipDeployment {
    pub contract_address: String,
    pub class_hash: String,
    pub deploy_block: u64,
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
}

/// Shared application state, wrapped in `Arc` by Axum.
pub struct AppState {
    pub config: Config,
    pub rpc: StarknetRpc,
    pub sessions: DashMap<String, Session>,
    pub coinflip: RwLock<Option<CoinFlipDeployment>>,
    /// Pending bet commitments keyed by session_id.
    pub commitments: DashMap<String, BetCommitment>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let rpc = StarknetRpc::new(&config.rpc_url);
        Self {
            config,
            rpc,
            sessions: DashMap::new(),
            coinflip: RwLock::new(None),
            commitments: DashMap::new(),
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
