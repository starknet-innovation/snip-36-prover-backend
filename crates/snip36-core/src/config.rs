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
    /// STRK fee-token address (chain-specific; devnet uses a different address than sepolia/mainnet).
    pub strk_token: String,
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

        let rpc_url = std::env::var("STARKNET_RPC_URL")
            .map_err(|_| ConfigError::Missing("STARKNET_RPC_URL"))?;
        let account_address = std::env::var("STARKNET_ACCOUNT_ADDRESS")
            .or_else(|_| std::env::var("MASTER_ACCOUNT_ADDRESS"))
            .map_err(|_| ConfigError::Missing("STARKNET_ACCOUNT_ADDRESS"))?;
        let private_key = std::env::var("STARKNET_PRIVATE_KEY")
            .or_else(|_| std::env::var("MASTER_PRIVATE_KEY"))
            .map_err(|_| ConfigError::Missing("STARKNET_PRIVATE_KEY"))?;
        let chain_id = std::env::var("STARKNET_CHAIN_ID").unwrap_or_else(|_| "SN_SEPOLIA".into());
        let gateway_url = std::env::var("STARKNET_GATEWAY_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let strk_token = std::env::var("STARKNET_STRK_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| crate::types::STRK_TOKEN.to_string());

        let project_dir = std::env::var("SNIP36_PROJECT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let output_dir = project_dir.join("output");
        let deps_dir = project_dir.join("deps");

        Ok(Self {
            rpc_url,
            account_address,
            private_key,
            chain_id,
            gateway_url,
            strk_token,
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

    /// Path to the virtual-OS runner binary.
    ///
    /// The canonical binary is `starknet_transaction_prover`; prebuilt bundles
    /// also include `starknet_os_runner` as a compatibility alias. Return
    /// whichever is present, preserving the historical alias as the fallback
    /// for the "not found" error path.
    pub fn runner_bin(&self) -> PathBuf {
        let release = self.deps_dir.join("sequencer/target/release");
        let tx_prover = release.join("starknet_transaction_prover");
        if tx_prover.exists() {
            return tx_prover;
        }
        release.join("starknet_os_runner")
    }

    /// Validated gateway endpoint for proof submission. Keep access parameters out of errors.
    pub fn gateway_submit_url(&self) -> Result<String, ConfigError> {
        let gateway = self
            .gateway_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or(ConfigError::Missing("STARKNET_GATEWAY_URL"))?;
        let mut url = reqwest::Url::parse(gateway).map_err(|_| ConfigError::InvalidGatewayUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ConfigError::InvalidGatewayUrl);
        }
        url.set_path(&format!(
            "{}/gateway/add_transaction",
            url.path().trim_end_matches('/')
        ));
        url.set_fragment(None);
        Ok(url.to_string())
    }

    /// Root for compiler sidecar binaries used by the sequencer runner.
    ///
    /// Sequencer v0.14.4 resolves `starknet-sierra-compile` from
    /// `$CARGO_TOOLS_ROOT/<binary>-<version>/bin/<binary>`. Keep this under
    /// `deps/` so prebuilt bundles are relocatable and do not depend on the
    /// user's global cargo home.
    pub fn compiler_tools_dir(&self) -> PathBuf {
        self.deps_dir.join("compiler-tools")
    }

    /// Find an installed starknet-sierra-compile sidecar in compiler_tools_dir.
    pub fn sierra_compiler_bin(&self) -> Option<PathBuf> {
        let root = self.compiler_tools_dir();
        let entries = std::fs::read_dir(root).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("starknet-sierra-compile-") {
                continue;
            }
            let binary = path.join("bin/starknet-sierra-compile");
            if binary.exists() {
                return Some(binary);
            }
        }
        None
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
    #[error("STARKNET_GATEWAY_URL must be an absolute HTTP(S) URL")]
    InvalidGatewayUrl,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &Path) -> Config {
        Config {
            rpc_url: "http://localhost:1".into(),
            account_address: "0x1".into(),
            private_key: "0x1".into(),
            chain_id: "SN_SEPOLIA".into(),
            gateway_url: None,
            strk_token: "0x1".into(),
            project_dir: root.into(),
            output_dir: root.join("output"),
            deps_dir: root.join("deps"),
        }
    }

    #[test]
    fn canonical_runner_takes_precedence_over_stale_alias() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let alias = config.runner_bin();
        std::fs::create_dir_all(alias.parent().unwrap()).unwrap();
        std::fs::write(&alias, b"old").unwrap();
        assert_eq!(config.runner_bin(), alias);
        let canonical = alias.with_file_name("starknet_transaction_prover");
        std::fs::write(&canonical, b"new").unwrap();
        assert_eq!(config.runner_bin(), canonical);
        std::fs::remove_file(alias).unwrap();
        assert_eq!(config.runner_bin(), canonical);
    }

    #[test]
    fn gateway_is_required_and_must_be_absolute_http() {
        let mut config = config(Path::new("."));
        assert!(matches!(
            config.gateway_submit_url(),
            Err(ConfigError::Missing(_))
        ));
        for invalid in ["", "  ", "/gateway", "ftp://example.com"] {
            config.gateway_url = Some(invalid.into());
            assert!(config.gateway_submit_url().is_err());
        }
        config.gateway_url = Some("https://example.com/base/?key=test#fragment".into());
        assert_eq!(
            config.gateway_submit_url().unwrap(),
            "https://example.com/base/gateway/add_transaction?key=test"
        );
    }
}
