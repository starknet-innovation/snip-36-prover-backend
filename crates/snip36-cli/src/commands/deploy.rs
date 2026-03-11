use clap::{Args, Subcommand};
use color_eyre::eyre::{bail, Result, WrapErr};
use tracing::info;

use snip36_core::types::OZ_ACCOUNT_CLASS_HASH;
use snip36_core::Config;

use super::{format_cmd_output, parse_hex_from_output, parse_long_hex};

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

    /// Declare and deploy a counter contract
    Counter {
        /// Salt for deployment (random if not given)
        #[arg(long)]
        salt: Option<String>,
    },
}

pub async fn run(args: DeployArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    match args.mode {
        DeployMode::Account { public_key, salt } => {
            deploy_account(&public_key, salt.as_deref(), env_file).await
        }
        DeployMode::Counter { salt } => deploy_counter(salt.as_deref(), env_file).await,
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
            "playground-master",
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

async fn deploy_counter(
    salt: Option<&str>,
    env_file: Option<&std::path::Path>,
) -> Result<()> {
    let config = Config::from_env(env_file)?;
    let contracts_dir = config.contracts_dir();

    info!("=== Deploy Counter Contract ===");

    // Declare
    info!("Declaring counter class...");
    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            "playground-master",
            "declare",
            "--contract-name",
            "Counter",
            "--url",
            &config.rpc_url,
        ])
        .current_dir(&contracts_dir)
        .output()
        .await
        .wrap_err("failed to run sncast declare")?;

    let declare_combined = format_cmd_output(&declare_output);
    info!("sncast declare output:\n{declare_combined}");

    // Try to extract class hash
    let class_hash = parse_hex_from_output("class_hash", &declare_combined)
        .or_else(|| parse_long_hex(&declare_combined));

    let class_hash = class_hash.ok_or_else(|| eyre::eyre!("declare failed: {declare_combined}"))?;
    info!("  Class hash: {class_hash}");

    // Deploy with salt
    let deploy_salt = match salt {
        Some(s) => s.to_string(),
        None => {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let bytes: [u8; 16] = rng.gen();
            format!("0x{}", hex::encode(bytes))
        }
    };

    info!("Deploying counter with salt {deploy_salt}...");
    let deploy_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            "playground-master",
            "deploy",
            "--class-hash",
            &class_hash,
            "--salt",
            &deploy_salt,
            "--url",
            &config.rpc_url,
        ])
        .output()
        .await
        .wrap_err("failed to run sncast deploy")?;

    let deploy_combined = format_cmd_output(&deploy_output);
    info!("sncast deploy output:\n{deploy_combined}");

    let contract_address = parse_hex_from_output("contract_address", &deploy_combined);
    let tx_hash = parse_hex_from_output("transaction_hash", &deploy_combined);

    match contract_address {
        Some(addr) => {
            info!("Counter deployed:");
            info!("  Class hash:       {class_hash}");
            info!("  Contract address: {addr}");
            if let Some(tx) = tx_hash {
                info!("  tx_hash:          {tx}");
            }
            Ok(())
        }
        None => bail!("deploy failed: {deploy_combined}"),
    }
}
