use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre::{Result, WrapErr};
use starknet_types_core::felt::Felt;
use tracing::info;

use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::StarknetRpc;
use snip36_core::signing::{felt_from_hex, sign_and_build_payload};
use snip36_core::types::{ResourceBounds, SubmitParams};
use snip36_core::Config;

#[derive(Args)]
pub struct SubmitArgs {
    /// Path to base64 proof file
    #[arg(long)]
    proof: PathBuf,

    /// Path to proof_facts JSON file
    #[arg(long)]
    proof_facts: PathBuf,

    /// Calldata as comma-separated hex values
    #[arg(long)]
    calldata: String,

    /// Contract address (for logging)
    #[arg(long)]
    contract_address: String,
}

pub async fn run(args: SubmitArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    let config = Config::from_env(env_file)?;

    info!("=== Sign and Submit SNIP-36 Proof ===");

    // Read proof base64
    let proof_b64 = tokio::fs::read_to_string(&args.proof)
        .await
        .wrap_err_with(|| format!("failed to read proof file: {}", args.proof.display()))?
        .trim()
        .to_string();

    // Read proof facts
    let proof_facts_str = tokio::fs::read_to_string(&args.proof_facts)
        .await
        .wrap_err_with(|| {
            format!(
                "failed to read proof_facts file: {}",
                args.proof_facts.display()
            )
        })?;
    let proof_facts_hex = parse_proof_facts_json(&proof_facts_str)
        .map_err(|e| eyre::eyre!("failed to parse proof_facts: {e}"))?;
    let proof_facts: Vec<Felt> = proof_facts_hex
        .iter()
        .map(|h| felt_from_hex(h).map_err(|e| eyre::eyre!(e)))
        .collect::<Result<_>>()?;

    // Parse calldata
    let calldata: Vec<Felt> = args
        .calldata
        .split(',')
        .map(|h| felt_from_hex(h.trim()).map_err(|e| eyre::eyre!(e)))
        .collect::<Result<_>>()?;

    // Fetch nonce
    let rpc = StarknetRpc::new(&config.rpc_url);
    let nonce = rpc.get_nonce(&config.account_address).await?;
    info!("  Nonce: {:#x}", nonce);

    let sender_address =
        felt_from_hex(&config.account_address).map_err(|e| eyre::eyre!(e))?;
    let private_key =
        felt_from_hex(&config.private_key).map_err(|e| eyre::eyre!(e))?;
    let chain_id = config.chain_id_felt()?;

    let resource_bounds = ResourceBounds::default();

    let params = SubmitParams {
        sender_address,
        private_key,
        calldata,
        proof_base64: proof_b64,
        proof_facts,
        nonce: Felt::from(nonce),
        chain_id,
        resource_bounds,
    };

    // Sign and build payload
    let (tx_hash, invoke_tx) =
        sign_and_build_payload(&params).map_err(|e| eyre::eyre!("signing failed: {e}"))?;

    info!("  Tx hash (with proof_facts): {:#x}", tx_hash);
    info!("Submitting INVOKE with proof via RPC...");
    info!("  Sender:  {}", config.account_address);
    info!("  Nonce:   {:#x}", nonce);
    info!("  RPC:     {}", config.rpc_url);

    // Submit via starknet_addInvokeTransaction
    let rpc_tx_hash = rpc.add_invoke_transaction(invoke_tx).await?;

    info!("SUCCESS: tx_hash = {rpc_tx_hash}");

    Ok(())
}
