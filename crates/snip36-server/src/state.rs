use dashmap::DashMap;
use snip36_core::config::Config;
use snip36_core::rpc::StarknetRpc;
use snip36_core::types::Session;
use tokio::sync::{Mutex, RwLock};

pub use crate::coinflip_state::{
    BankDeployment, BetCommitment, CoinFlipDeployment, PersistedDeployments,
};

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
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
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
