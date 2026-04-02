use clap::{Args, Subcommand};
use color_eyre::eyre::{bail, Result, WrapErr};
use tracing::info;

use snip36_core::types::OZ_ACCOUNT_CLASS_HASH;
use snip36_core::Config;

use super::{format_cmd_output, parse_hex_from_output};

#[derive(Args)]
pub struct DeployArgs {
    #[command(subcommand)]
    pub mode: DeployMode,
}

#[derive(Subcommand)]
pub enum DeployMode {
    /// Deploy an OpenZeppelin account contract
    Account {
        /// Public key for the account
        #[arg(long)]
        public_key: String,

        /// Salt for deterministic address (defaults to public_key)
        #[arg(long)]
        salt: Option<String>,
    },
}

pub async fn run(args: DeployArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    match args.mode {
        DeployMode::Account { public_key, salt } => {
            deploy_account(&public_key, salt.as_deref(), env_file).await
        }
    }
}

async fn deploy_account(
    public_key: &str,
    salt: Option<&str>,
    env_file: Option<&std::path::Path>,
) -> Result<()> {
    let config = Config::from_env(env_file)?;
    let salt = salt.unwrap_or(public_key);

    info!("=== Deploy OZ Account ===");
    info!("  Public key: {public_key}");
    info!("  Salt:       {salt}");
    info!("  Class hash: {OZ_ACCOUNT_CLASS_HASH}");

    let output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &config.sncast_account(),
            "deploy",
            "--class-hash",
            OZ_ACCOUNT_CLASS_HASH,
            "--constructor-calldata",
            public_key,
            "--salt",
            salt,
            "--url",
            &config.rpc_url,
        ])
        .output()
        .await
        .wrap_err("failed to run sncast")?;

    let combined = format_cmd_output(&output);
    info!("sncast output:\n{combined}");

    let address = parse_hex_from_output("contract_address", &combined);
    let tx_hash = parse_hex_from_output("transaction_hash", &combined);

    match address {
        Some(addr) => {
            info!("Account deployed:");
            info!("  Address: {addr}");
            if let Some(tx) = tx_hash {
                info!("  tx_hash: {tx}");
            }
            Ok(())
        }
        None => bail!("account deploy failed: {combined}"),
    }
}
