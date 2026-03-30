use std::sync::atomic::{AtomicU32, Ordering};

use clap::Args;
use color_eyre::eyre::Result;
use tracing::{error, info};

use snip36_core::rpc::StarknetRpc;
use snip36_core::types::{BALANCE_OF_SELECTOR, OZ_ACCOUNT_CLASS_HASH, STRK_TOKEN};
use snip36_core::Config;

use crate::selectors::GET_COUNTER_SELECTOR;

use snip36_core::cli_util::{format_cmd_output, parse_hex_from_output, parse_long_hex};

static PASSED: AtomicU32 = AtomicU32::new(0);
static FAILED: AtomicU32 = AtomicU32::new(0);

fn check_pass(name: &str, detail: &str) {
    PASSED.fetch_add(1, Ordering::Relaxed);
    if detail.is_empty() {
        info!("  PASS: {name}");
    } else {
        info!("  PASS: {name} ({detail})");
    }
}

fn check_fail(name: &str, detail: &str) {
    FAILED.fetch_add(1, Ordering::Relaxed);
    if detail.is_empty() {
        error!("  FAIL: {name}");
    } else {
        error!("  FAIL: {name} -- {detail}");
    }
}

#[derive(Args)]
pub struct HealthArgs {
    /// Skip the full flow check (only run RPC checks)
    #[arg(long)]
    pub quick: bool,
}

pub async fn run(args: HealthArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    let config = Config::from_env(env_file)?;

    // Reset counters
    PASSED.store(0, Ordering::Relaxed);
    FAILED.store(0, Ordering::Relaxed);

    info!("=== SNIP-36 Playground Health Check ===");
    info!(
        "  RPC:     {}",
        config.rpc_url
    );
    info!(
        "  Account: {}...{}",
        &config.account_address[..10.min(config.account_address.len())],
        &config.account_address[config.account_address.len().saturating_sub(6)..]
    );

    let rpc = StarknetRpc::new(&config.rpc_url);

    // Check 1: RPC reachable
    check_rpc(&rpc).await;

    // Check 2: Master account balance
    check_balance(&rpc, &config.account_address).await;

    // Check 3: OZ class declared
    check_oz_class(&rpc).await;

    // Check 4: Full flow
    if !args.quick && !config.private_key.is_empty() {
        check_full_flow(&config, &rpc).await;
    } else if args.quick {
        info!("\n-- Check 4: Skipped (--quick) --");
    } else {
        info!("\n-- Check 4: Skipped (no STARKNET_PRIVATE_KEY) --");
    }

    let passed = PASSED.load(Ordering::Relaxed);
    let failed = FAILED.load(Ordering::Relaxed);

    info!("");
    info!("==========================================");
    info!("  Passed: {passed}");
    info!("  Failed: {failed}");
    info!("==========================================");

    if failed > 0 {
        info!("  RESULT: {failed} CHECK(S) FAILED");
        std::process::exit(1);
    } else {
        info!("  RESULT: ALL CHECKS PASSED");
        Ok(())
    }
}

async fn check_rpc(rpc: &StarknetRpc) {
    info!("\n-- Check 1: RPC node reachable --");
    match rpc.block_number().await {
        Ok(block) if block > 0 => {
            check_pass("RPC reachable", &format!("block {block}"));
        }
        Ok(block) => {
            check_fail("RPC reachable", &format!("unexpected block number: {block}"));
            return;
        }
        Err(e) => {
            check_fail("RPC reachable", &e.to_string());
            return;
        }
    }

    match rpc.chain_id().await {
        Ok(chain_id) => check_pass("Chain ID", &chain_id),
        Err(e) => check_fail("Chain ID", &e.to_string()),
    }
}

async fn check_balance(rpc: &StarknetRpc, master_address: &str) {
    info!("\n-- Check 2: Master account STRK balance --");
    match rpc
        .starknet_call(STRK_TOKEN, BALANCE_OF_SELECTOR, &[master_address])
        .await
    {
        Ok(values) => {
            if values.is_empty() {
                check_fail("Balance check", "empty result");
                return;
            }
            let low = u128::from_str_radix(values[0].trim_start_matches("0x"), 16).unwrap_or(0);
            let high = if values.len() > 1 {
                u128::from_str_radix(values[1].trim_start_matches("0x"), 16).unwrap_or(0)
            } else {
                0
            };
            let balance_wei = low + (high << 128);
            let balance_strk = balance_wei as f64 / 1e18;

            if balance_strk >= 1.0 {
                check_pass("STRK balance", &format!("{balance_strk:.2} STRK"));
            } else {
                check_fail(
                    "STRK balance too low",
                    &format!("{balance_strk:.2} STRK < 1.0 STRK minimum"),
                );
            }
        }
        Err(e) => check_fail("Balance check", &e.to_string()),
    }
}

async fn check_oz_class(rpc: &StarknetRpc) {
    info!("\n-- Check 3: OZ Account class declared --");
    match rpc.get_class(OZ_ACCOUNT_CLASS_HASH).await {
        Ok(_) => check_pass(
            "OZ class declared",
            &format!("{}...", &OZ_ACCOUNT_CLASS_HASH[..18]),
        ),
        Err(e) => check_fail("OZ class declared", &e.to_string()),
    }
}

async fn check_full_flow(config: &Config, rpc: &StarknetRpc) {
    info!("\n-- Check 4: Full playground flow --");

    // Check prerequisites
    for cmd in ["sncast", "scarb"] {
        let check = tokio::process::Command::new("which")
            .arg(cmd)
            .output()
            .await;
        if check.map(|o| !o.status.success()).unwrap_or(true) {
            check_fail(&format!("{cmd} not in PATH"), "skipping full flow");
            return;
        }
    }

    let sncast_account = "ci-health-check";

    // Import account
    let _ = tokio::process::Command::new("sncast")
        .args([
            "account",
            "import",
            "--name",
            sncast_account,
            "--address",
            &config.account_address,
            "--private-key",
            &config.private_key,
            "--type",
            "oz",
            "--url",
            &config.rpc_url,
            "--silent",
        ])
        .output()
        .await;

    // 4a: Compile + declare counter
    info!("  Compiling counter contract...");
    let contracts_dir = config.contracts_dir();
    let build = tokio::process::Command::new("scarb")
        .arg("build")
        .current_dir(&contracts_dir)
        .output()
        .await;

    match build {
        Ok(output) if output.status.success() => {
            check_pass("Compile counter", "");
        }
        Ok(output) => {
            let out = format_cmd_output(&output);
            check_fail("Compile counter", &out[..out.len().min(500)]);
            return;
        }
        Err(e) => {
            check_fail("Compile counter", &e.to_string());
            return;
        }
    }

    info!("  Declaring counter class...");
    let declare = tokio::process::Command::new("sncast")
        .args([
            "--account",
            sncast_account,
            "declare",
            "--contract-name",
            "Counter",
            "--url",
            &config.rpc_url,
        ])
        .current_dir(&contracts_dir)
        .output()
        .await;

    let class_hash = match declare {
        Ok(output) => {
            let combined = format_cmd_output(&output);
            parse_hex_from_output("class_hash", &combined)
                .or_else(|| parse_long_hex(&combined))
        }
        Err(e) => {
            check_fail("Declare counter", &e.to_string());
            return;
        }
    };

    let class_hash = match class_hash {
        Some(h) => {
            check_pass("Declare counter", &format!("{}...", &h[..h.len().min(18)]));
            h
        }
        None => {
            check_fail("Declare counter", "could not parse class_hash");
            return;
        }
    };

    // 4b: Deploy counter
    info!("  Deploying counter contract...");
    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));
    let deploy = tokio::process::Command::new("sncast")
        .args([
            "--account",
            sncast_account,
            "deploy",
            "--class-hash",
            &class_hash,
            "--salt",
            &salt,
            "--url",
            &config.rpc_url,
        ])
        .output()
        .await;

    let (contract_address, deploy_tx) = match deploy {
        Ok(output) => {
            let combined = format_cmd_output(&output);
            let addr = parse_hex_from_output("contract_address", &combined);
            let tx = parse_hex_from_output("transaction_hash", &combined);
            (addr, tx)
        }
        Err(e) => {
            check_fail("Deploy counter", &e.to_string());
            return;
        }
    };

    let contract_address = match contract_address {
        Some(addr) => {
            check_pass("Deploy counter", &format!("{}...", &addr[..addr.len().min(18)]));
            addr
        }
        None => {
            check_fail("Deploy counter", "could not parse contract_address");
            return;
        }
    };

    if let Some(tx) = deploy_tx {
        match rpc.wait_for_tx(&tx, 120, 3).await {
            Ok(receipt) => {
                if let Some(bn) = snip36_core::rpc::receipt_block_number(&receipt) {
                    let _ = rpc.wait_for_block_after(bn, 120, 3).await;
                    check_pass("Deploy tx confirmed", &format!("block {bn}"));
                }
            }
            Err(_) => {
                check_fail("Deploy tx confirmation", "timeout");
                return;
            }
        }
    }

    // 4c: Invoke increment
    info!("  Invoking increment(1)...");
    let invoke = tokio::process::Command::new("sncast")
        .args([
            "--account",
            sncast_account,
            "invoke",
            "--contract-address",
            &contract_address,
            "--function",
            "increment",
            "--calldata",
            "0x1",
            "--url",
            &config.rpc_url,
        ])
        .output()
        .await;

    let invoke_tx = match invoke {
        Ok(output) => {
            let combined = format_cmd_output(&output);
            parse_hex_from_output("transaction_hash", &combined)
        }
        Err(e) => {
            check_fail("Invoke increment", &e.to_string());
            return;
        }
    };

    let invoke_tx = match invoke_tx {
        Some(tx) => {
            check_pass("Invoke increment", &format!("{}...", &tx[..tx.len().min(18)]));
            tx
        }
        None => {
            check_fail("Invoke increment", "could not parse tx hash");
            return;
        }
    };

    match rpc.wait_for_tx(&invoke_tx, 120, 3).await {
        Ok(receipt) => {
            check_pass("Invoke tx confirmed", "");
            // Wait for next block so `latest` state reflects the invoke
            if let Some(bn) = snip36_core::rpc::receipt_block_number(&receipt) {
                let _ = rpc.wait_for_block_after(bn, 120, 3).await;
            }
        }
        Err(_) => {
            check_fail("Invoke tx confirmation", "timeout");
            return;
        }
    }

    // 4d: Read counter
    info!("  Reading counter value...");
    match rpc
        .starknet_call(&contract_address, GET_COUNTER_SELECTOR, &[])
        .await
    {
        Ok(values) => {
            let counter = values
                .first()
                .and_then(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0);
            if counter >= 1 {
                check_pass("Counter value", &counter.to_string());
            } else {
                check_fail("Counter value", &format!("expected >= 1, got {counter}"));
            }
        }
        Err(e) => check_fail("Read counter", &e.to_string()),
    }
}
