//! E2E suite orchestrated against a locally-spawned starknet-devnet.
//!
//! Spawns starknet-devnet with a deterministic seed, wires its predeployed
//! account / RPC URL / chain-id / STRK fee token into the three existing
//! e2e flows (counter, messages, coinflip), and runs them sequentially.
//! Submission goes through devnet's RPC (no gateway).

use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre::{bail, Result};
use tracing::{error, info};

use super::devnet;

#[derive(Args)]
pub struct E2eDevnetArgs {
    /// Path to the starknet-devnet binary (default: resolve via PATH)
    #[arg(long, default_value = "starknet-devnet")]
    pub devnet_bin: String,

    /// Port for starknet-devnet
    #[arg(long, default_value = "5050")]
    pub port: u16,

    /// Seed for predeployed accounts (deterministic across runs)
    #[arg(long, default_value = "0")]
    pub seed: u64,

    /// Comma-separated list of flows to run: counter, messages, coinflip
    #[arg(long, default_value = "counter,messages,coinflip")]
    pub flows: String,

    /// Remote prover URL (skip local starknet_os_runner)
    #[arg(long)]
    pub prover_url: Option<String>,

    /// Output root directory for artifacts (each flow gets a subdir)
    #[arg(long, default_value = "output/e2e-devnet")]
    pub output_dir: PathBuf,

    /// Leave devnet running after the suite finishes (useful for debugging)
    #[arg(long)]
    pub keep_alive: bool,
}

pub async fn run(args: E2eDevnetArgs, _env_file: Option<&std::path::Path>) -> Result<()> {
    info!("=== SNIP-36 E2E Suite (devnet) ===");

    // Spawn devnet and discover its parameters.
    let devnet_handle = devnet::spawn(&args.devnet_bin, args.port, args.seed).await?;
    let url = devnet_handle.url().to_string();

    let account = devnet::fetch_predeployed_account(&url).await?;
    info!("  Predeployed account: {}", account.address);

    let chain_id = devnet::fetch_chain_id(&url).await?;
    info!("  Chain id: {chain_id}");

    // starknet-devnet predeploys STRK at the canonical mainnet/sepolia address,
    // which matches Config::strk_token's default, so no STARKNET_STRK_TOKEN override
    // is needed here.

    // Inject config via env vars so each flow's Config::from_env picks them up.
    std::env::set_var("STARKNET_RPC_URL", &url);
    std::env::set_var("STARKNET_ACCOUNT_ADDRESS", &account.address);
    std::env::set_var("STARKNET_PRIVATE_KEY", &account.private_key);
    std::env::set_var("STARKNET_CHAIN_ID", &chain_id);
    // Ensure submission goes through RPC, not a gateway.
    std::env::remove_var("STARKNET_GATEWAY_URL");

    tokio::fs::create_dir_all(&args.output_dir).await?;

    let flows: Vec<&str> = args.flows.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let mut results: Vec<(String, Result<()>)> = Vec::new();

    for flow in &flows {
        info!("");
        info!("=== Running flow: {flow} ===");
        let flow_output = args.output_dir.join(flow);
        let result = match *flow {
            "counter" => run_counter(&args, &flow_output).await,
            "messages" => run_messages(&args, &flow_output).await,
            "coinflip" => run_coinflip(&args, &flow_output).await,
            other => {
                error!("  Unknown flow: {other}");
                Err(color_eyre::eyre::eyre!("unknown flow: {other}"))
            }
        };
        results.push((flow.to_string(), result));
    }

    info!("");
    info!("=== Suite summary ===");
    let mut failures = 0;
    for (flow, result) in &results {
        match result {
            Ok(()) => info!("  PASS: {flow}"),
            Err(e) => {
                failures += 1;
                error!("  FAIL: {flow}: {e:#}");
            }
        }
    }

    if args.keep_alive {
        info!("  --keep-alive set: devnet still running at {url}");
        devnet_handle.detach();
    } else {
        drop(devnet_handle);
    }

    if failures > 0 {
        bail!("{failures} flow(s) failed out of {}", results.len());
    }
    Ok(())
}

async fn run_counter(args: &E2eDevnetArgs, output_dir: &std::path::Path) -> Result<()> {
    let counter_args = snip36_counter::e2e::E2eArgs {
        prover_url: args.prover_url.clone(),
        output_dir: output_dir.to_path_buf(),
        snos_blocks: 1,
        counter_increments: 1,
        increments_per_snos: 1,
        prove_only: false,
    };
    snip36_counter::e2e::run(counter_args, None).await
}

async fn run_messages(args: &E2eDevnetArgs, output_dir: &std::path::Path) -> Result<()> {
    let messages_args = snip36_messages::e2e::E2eMessagesArgs {
        prover_url: args.prover_url.clone(),
        output_dir: output_dir.to_path_buf(),
        prove_only: false,
        to_address: "0x123".to_string(),
        payload: "0x1,0x2,0x3".to_string(),
    };
    snip36_messages::e2e::run(messages_args, None).await
}

async fn run_coinflip(args: &E2eDevnetArgs, output_dir: &std::path::Path) -> Result<()> {
    let coinflip_args = snip36_coinflip::e2e::E2eCoinflipArgs {
        prover_url: args.prover_url.clone(),
        output_dir: output_dir.to_path_buf(),
        prove_only: false,
        bet: 0,
    };
    snip36_coinflip::e2e::run(coinflip_args, None).await
}
