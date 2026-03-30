use dashmap::DashMap;
use snip36_core::config::Config;
use snip36_core::rpc::StarknetRpc;
use snip36_core::types::Session;
use tokio::sync::Mutex;

/// Shared application state for generic SNIP-36 routes.
///
/// Application-specific state (e.g. CoinFlip deployments) lives in app crates
/// and is composed at the binary level (see `apps/playground`).
pub struct AppState {
    pub config: Config,
    pub rpc: StarknetRpc,
    pub sessions: DashMap<String, Session>,
    /// Mutex for serializing sncast invocations that share the master account nonce.
    /// Currently only used by coinflip routes; deploy-account and fund use
    /// separate nonces or are unlikely to race in practice.
    pub sncast_lock: Mutex<()>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let rpc = StarknetRpc::new(&config.rpc_url);
        Self {
            config,
            rpc,
            sessions: DashMap::new(),
            sncast_lock: Mutex::new(()),
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
