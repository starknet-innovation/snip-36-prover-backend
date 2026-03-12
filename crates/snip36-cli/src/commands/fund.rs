use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use tracing::info;

use snip36_core::types::STRK_TOKEN;
use snip36_core::Config;

use super::{format_cmd_output, parse_hex_from_output};

#[derive(Args)]
pub struct FundArgs {
    /// Target address to fund
    #[arg(long)]
    to: String,

    /// Amount to transfer in wei (default: 10 STRK = 10^19 wei)
    #[arg(long, default_value = "10000000000000000000")]
    amount: u128,
}

pub async fn run(args: FundArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    let config = Config::from_env(env_file)?;

    let amount_low = args.amount & ((1u128 << 128) - 1);
    let amount_high = args.amount >> 128;

    info!("=== Transfer STRK ===");
    info!("  To:     {}", args.to);
    info!("  Amount: {} wei ({:.2} STRK)", args.amount, args.amount as f64 / 1e18);

    let calldata = format!("{} {:#x} {:#x}", args.to, amount_low, amount_high);

    let output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &config.sncast_account(),
            "invoke",
            "--contract-address",
            STRK_TOKEN,
            "--function",
            "transfer",
            "--calldata",
            &calldata,
            "--url",
            &config.rpc_url,
        ])
        .output()
        .await
        .wrap_err("failed to run sncast")?;

    let combined = format_cmd_output(&output);
    info!("sncast output:\n{combined}");

    if !output.status.success() {
        bail!("sncast invoke failed: {combined}");
    }

    let tx_hash = parse_hex_from_output("transaction_hash", &combined);
    match tx_hash {
        Some(hash) => {
            info!("SUCCESS: transfer tx_hash = {hash}");
            Ok(())
        }
        None => bail!("could not parse transaction hash from sncast output"),
    }
}
