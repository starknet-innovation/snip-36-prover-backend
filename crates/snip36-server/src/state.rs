use dashmap::DashMap;
use snip36_core::config::Config;
use snip36_core::rpc::StarknetRpc;
use snip36_core::types::Session;

/// Shared application state, wrapped in `Arc` by Axum.
pub struct AppState {
    pub config: Config,
    pub rpc: StarknetRpc,
    pub sessions: DashMap<String, Session>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let rpc = StarknetRpc::new(&config.rpc_url);
        Self {
            config,
            rpc,
            sessions: DashMap::new(),
        }
    }

    /// Get or create a session for the given ID.
    pub fn get_session(&self, session_id: &str) -> Session {
        self.sessions
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    /// Update a session in place.
    pub fn update_session(&self, session_id: &str, session: Session) {
        self.sessions.insert(session_id.to_string(), session);
    }
}
