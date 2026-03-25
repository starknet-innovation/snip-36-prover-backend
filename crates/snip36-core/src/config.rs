use std::path::{Path, PathBuf};

/// Typed configuration loaded from environment variables / .env file.
#[derive(Debug, Clone)]
pub struct Config {
    /// Starknet JSON-RPC endpoint URL.
    pub rpc_url: String,
    /// Master/sender account address (hex).
    pub account_address: String,
    /// Private key for signing (hex).
    pub private_key: String,
    /// Starknet chain ID string (e.g. "SN_SEPOLIA").
    pub chain_id: String,
    /// Gateway URL for proof submission (bypasses RPC node).
    pub gateway_url: Option<String>,
    /// Project root directory.
    pub project_dir: PathBuf,
    /// Output directory for proofs and artifacts.
    pub output_dir: PathBuf,
    /// Path to the `deps/` directory (cloned repos, built binaries).
    pub deps_dir: PathBuf,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Optionally reads a `.env` file from the given path first.
    pub fn from_env(env_file: Option<&Path>) -> Result<Self, ConfigError> {
        if let Some(path) = env_file {
            if path.exists() {
                dotenvy::from_path(path).ok();
            }
        } else {
            dotenvy::dotenv().ok();
        }

        let rpc_url =
            std::env::var("STARKNET_RPC_URL").map_err(|_| ConfigError::Missing("STARKNET_RPC_URL"))?;
        let account_address = std::env::var("STARKNET_ACCOUNT_ADDRESS")
            .or_else(|_| std::env::var("MASTER_ACCOUNT_ADDRESS"))
            .map_err(|_| ConfigError::Missing("STARKNET_ACCOUNT_ADDRESS"))?;
        let private_key = std::env::var("STARKNET_PRIVATE_KEY")
            .or_else(|_| std::env::var("MASTER_PRIVATE_KEY"))
            .map_err(|_| ConfigError::Missing("STARKNET_PRIVATE_KEY"))?;
        let chain_id =
            std::env::var("STARKNET_CHAIN_ID").unwrap_or_else(|_| "SN_SEPOLIA".into());
        let gateway_url = std::env::var("STARKNET_GATEWAY_URL")
            .ok()
            .filter(|s| !s.is_empty());

        let project_dir = std::env::var("SNIP36_PROJECT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            });
        let output_dir = project_dir.join("output");
        let deps_dir = project_dir.join("deps");

        Ok(Self {
            rpc_url,
            account_address,
            private_key,
            chain_id,
            gateway_url,
            project_dir,
            output_dir,
            deps_dir,
        })
    }

    /// Encode the chain_id string as a Starknet felt (short string encoding).
    ///
    /// Returns an error if chain_id exceeds 31 bytes (max Starknet short string).
    pub fn chain_id_felt(&self) -> Result<starknet_types_core::felt::Felt, ConfigError> {
        let bytes = self.chain_id.as_bytes();
        if bytes.len() > 31 {
            return Err(ConfigError::InvalidChainId(format!(
                "chain_id '{}' is {} bytes, max is 31",
                self.chain_id,
                bytes.len()
            )));
        }
        let mut buf = [0u8; 32];
        let start = 32 - bytes.len();
        buf[start..].copy_from_slice(bytes);
        Ok(starknet_types_core::felt::Felt::from_bytes_be(&buf))
    }

    /// Path to the stwo-run-and-prove binary.
    pub fn prover_bin(&self) -> PathBuf {
        self.deps_dir.join("bin/stwo-run-and-prove")
    }

    /// Path to the starknet_transaction_prover binary.
    pub fn runner_bin(&self) -> PathBuf {
        self.deps_dir
            .join("sequencer/target/release/starknet_transaction_prover")
    }

    /// Path to the bootloader program JSON.
    pub fn bootloader_program(&self) -> PathBuf {
        self.deps_dir.join("bin/bootloader_program.json")
    }

    /// Path to the prover parameters JSON.
    pub fn prover_params(&self) -> PathBuf {
        self.project_dir.join("sample-input/prover_params.json")
    }

    /// Path to the test contracts directory.
    pub fn contracts_dir(&self) -> PathBuf {
        self.project_dir.join("tests/contracts")
    }

    /// The sncast account name for the master/funding account.
    pub fn sncast_account(&self) -> String {
        std::env::var("SNCAST_ACCOUNT").unwrap_or_else(|_| "playground-master".into())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    Missing(&'static str),
    #[error("invalid chain_id: {0}")]
    InvalidChainId(String),
}
