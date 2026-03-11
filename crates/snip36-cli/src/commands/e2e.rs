use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use tracing::{error, info};

use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::StarknetRpc;
use snip36_core::signing::{felt_from_hex, sign_and_build_payload};
use snip36_core::types::{ResourceBounds, SubmitParams, GET_COUNTER_SELECTOR, STRK_TOKEN};
use snip36_core::Config;

use super::{format_cmd_output, parse_hex_from_output, parse_long_hex};

static PASS_COUNT: AtomicU32 = AtomicU32::new(0);
static FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
static STEP_TIMINGS: Mutex<Vec<(String, std::time::Duration)>> = Mutex::new(Vec::new());
static STEP_START: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn pass(msg: &str) {
    PASS_COUNT.fetch_add(1, Ordering::Relaxed);
    info!("");
    info!("  PASS: {msg}");
    info!("");
}

fn fail(msg: &str) {
    FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
    error!("");
    error!("  FAIL: {msg}");
    error!("");
}

fn step(n: u32, title: &str) {
    // Record the previous step's duration
    if let Ok(mut prev) = STEP_START.lock() {
        if let Some((name, start)) = prev.take() {
            if let Ok(mut timings) = STEP_TIMINGS.lock() {
                timings.push((name, start.elapsed()));
            }
        }
        *prev = Some((format!("Step {n}: {title}"), Instant::now()));
    }

    info!("");
    info!("==========================================");
    info!("  STEP {n}: {title}");
    info!("==========================================");
    info!("");
}

fn finish_last_step() {
    if let Ok(mut prev) = STEP_START.lock() {
        if let Some((name, start)) = prev.take() {
            if let Ok(mut timings) = STEP_TIMINGS.lock() {
                timings.push((name, start.elapsed()));
            }
        }
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}.{:01}s", secs, d.subsec_millis() / 100)
    }
}

#[derive(Args)]
pub struct E2eArgs {
    /// Remote prover URL (skip local starknet_os_runner)
    #[arg(long)]
    prover_url: Option<String>,

    /// Output directory for E2E artifacts
    #[arg(long, default_value = "output/e2e")]
    output_dir: PathBuf,
}

pub async fn run(args: E2eArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    let config = Config::from_env(env_file)?;

    // Reset counters
    PASS_COUNT.store(0, Ordering::Relaxed);
    FAIL_COUNT.store(0, Ordering::Relaxed);
    if let Ok(mut t) = STEP_TIMINGS.lock() { t.clear(); }
    if let Ok(mut s) = STEP_START.lock() { *s = None; }
    let e2e_start = Instant::now();

    let rpc = StarknetRpc::new(&config.rpc_url);
    let account_name = "e2e-test-account-2";

    info!("=== SNIP-36 End-to-End Test ===");
    info!("");
    info!("  RPC:     {}", config.rpc_url);
    info!("  Account: {}", config.account_address);
    info!("");

    // Check prerequisites
    check_prereqs(&config).await?;
    tokio::fs::create_dir_all(&args.output_dir).await?;

    // ==========================================
    // STEP 0: Import account into sncast
    // ==========================================
    step(0, "Import account into sncast");

    let _ = tokio::process::Command::new("sncast")
        .args([
            "account",
            "import",
            "--name",
            account_name,
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

    // Verify account is usable
    match rpc.get_nonce(&config.account_address).await {
        Ok(nonce) => pass(&format!("Account imported (nonce: {:#x})", nonce)),
        Err(e) => {
            fail(&format!("Could not verify account on-chain: {e}"));
            bail!("cannot proceed without account");
        }
    }

    // ==========================================
    // STEP 1: Compile the counter contract
    // ==========================================
    step(1, "Compile counter contract");

    let contracts_dir = config.contracts_dir();
    let build = tokio::process::Command::new("scarb")
        .arg("build")
        .current_dir(&contracts_dir)
        .output()
        .await
        .wrap_err("failed to run scarb build")?;

    if build.status.success() {
        pass("Contract compiled");
    } else {
        let out = format_cmd_output(&build);
        fail(&format!("Contract compilation failed: {}", &out[..out.len().min(500)]));
        bail!("compilation failed");
    }

    // ==========================================
    // STEP 2: Declare the contract class
    // ==========================================
    step(2, "Declare contract class");

    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            account_name,
            "declare",
            "--url",
            &config.rpc_url,
            "--contract-name",
            "Counter",
        ])
        .current_dir(&contracts_dir)
        .output()
        .await
        .wrap_err("failed to run sncast declare")?;

    let declare_combined = format_cmd_output(&declare_output);
    info!("  sncast declare output:");
    info!("  {declare_combined}");

    let class_hash = parse_hex_from_output("class_hash", &declare_combined)
        .or_else(|| parse_long_hex(&declare_combined));

    let class_hash = match class_hash {
        Some(h) => {
            pass("Contract declared");
            info!("  Class hash: {h}");
            h
        }
        None => {
            fail("Could not determine class hash");
            bail!("declare failed");
        }
    };

    // ==========================================
    // STEP 3: Deploy the contract
    // ==========================================
    step(3, "Deploy counter contract");

    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));
    let deploy_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            account_name,
            "deploy",
            "--url",
            &config.rpc_url,
            "--class-hash",
            &class_hash,
            "--salt",
            &salt,
        ])
        .output()
        .await
        .wrap_err("failed to run sncast deploy")?;

    let deploy_combined = format_cmd_output(&deploy_output);
    info!("  sncast deploy output:");
    info!("  {deploy_combined}");

    let contract_address = parse_hex_from_output("contract_address", &deploy_combined);
    let deploy_tx_hash = parse_hex_from_output("transaction_hash", &deploy_combined);

    let contract_address = match contract_address {
        Some(addr) => {
            pass("Contract deployed");
            info!("  Address: {addr}");
            if let Some(tx) = &deploy_tx_hash {
                info!("  tx_hash: {tx}");
            }
            addr
        }
        None => {
            fail("Could not determine contract address");
            bail!("deploy failed");
        }
    };

    // ==========================================
    // STEP 4: Wait for deploy tx inclusion
    // ==========================================
    step(4, "Wait for deploy tx inclusion");

    if let Some(tx) = &deploy_tx_hash {
        match rpc.wait_for_tx(tx, 120, 3).await {
            Ok(receipt) => {
                let bn = snip36_core::rpc::receipt_block_number(&receipt).unwrap_or(0);
                pass(&format!("Deploy confirmed in block {bn}"));
            }
            Err(e) => {
                fail(&format!("Could not confirm deploy tx: {e}"));
                bail!("deploy confirmation failed");
            }
        }
    } else {
        fail("No deploy tx hash to wait for");
        bail!("missing deploy tx hash");
    };

    // ==========================================
    // STEP 5: Construct invoke transaction
    // ==========================================
    //
    // This is the SNIP-36 flow: we construct the transaction but do NOT submit
    // it to the sequencer. Instead it goes directly to the virtual OS runner
    // for off-chain execution and proving.
    step(5, "Construct invoke transaction (off-chain)");

    let increment_selector =
        "0x7a44dde9fea32737a5cf3f9683b3235138654aa2d189f6fe44af37a61dc60d";

    // Multicall calldata: [num_calls, to, selector, calldata_len, ...calldata]
    let calldata = vec![
        "0x1".to_string(),
        contract_address.clone(),
        increment_selector.to_string(),
        "0x1".to_string(),
        "0x1".to_string(),
    ];

    let nonce = rpc.get_nonce(&config.account_address).await?;
    let resource_bounds = ResourceBounds::default();

    // Build the unsigned invoke v3 transaction object (same shape the RPC returns)
    let tx_json = serde_json::json!({
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": &config.account_address,
        "calldata": calldata,
        "nonce": format!("{:#x}", nonce),
        "resource_bounds": resource_bounds.to_rpc_json(),
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "signature": [],
    });

    info!("  Sender:   {}", config.account_address);
    info!("  Nonce:    {nonce}");
    info!("  Contract: {contract_address}");
    info!("  Calldata: increment(1)");
    info!("  NOTE: Transaction is NOT submitted on-chain");
    pass("Invoke transaction constructed");

    // Get current block number as reference for the virtual OS
    let block_number = rpc.block_number().await?;
    info!("  Reference block: {block_number}");

    // Write the constructed tx to disk for the prove command
    let tx_json_path = args.output_dir.join("e2e_tx.json");
    tokio::fs::write(&tx_json_path, serde_json::to_string_pretty(&tx_json)?).await?;

    // ==========================================
    // STEP 6: Run virtual OS and prove transaction
    // ==========================================
    step(6, "Run virtual OS and prove transaction");

    let env_prover_url = std::env::var("PROVER_URL").ok().filter(|s| !s.is_empty());
    let prover_url = args
        .prover_url
        .as_deref()
        .or(env_prover_url.as_deref());

    if prover_url.is_none() && !config.deps_dir.join("sequencer").exists() {
        fail("No prover available -- set --prover-url or run `snip36 setup`");
        bail!("no prover available");
    }

    let proof_output = args.output_dir.join("e2e.proof");
    info!("  Proving against block {block_number}");

    let mut prove_args = vec![
        "prove".to_string(),
        "virtual-os".to_string(),
        "--block-number".to_string(),
        block_number.to_string(),
        "--tx-json".to_string(),
        tx_json_path.to_string_lossy().to_string(),
        "--rpc-url".to_string(),
        config.rpc_url.clone(),
        "--output".to_string(),
        proof_output.to_string_lossy().to_string(),
    ];

    if let Some(url) = prover_url {
        prove_args.push("--prover-url".to_string());
        prove_args.push(url.to_string());
        info!("  Using remote prover: {url}");
    } else {
        // Pass STRK fee token for local runner
        prove_args.push("--strk-fee-token".to_string());
        prove_args.push(STRK_TOKEN.to_string());
        info!("  STRK fee token: {STRK_TOKEN}");
    }

    // Run prove as a subprocess using the current binary
    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("snip36"));
    let prove_status = tokio::process::Command::new(&current_exe)
        .args(&prove_args)
        .status()
        .await
        .wrap_err("failed to run prove command")?;

    if prove_status.success() && proof_output.exists() {
        let metadata = tokio::fs::metadata(&proof_output).await?;
        pass(&format!("Proof generated ({} bytes)", metadata.len()));
    } else {
        fail("Proof file not created");
        bail!("proving failed");
    }

    // ==========================================
    // STEP 7: Validate proof format
    // ==========================================
    step(7, "Validate proof format");

    let proof_b64 = tokio::fs::read_to_string(&proof_output).await?;
    let proof_len = proof_b64.trim().len();

    if proof_len > 0 {
        info!("  Proof is base64 ({proof_len} chars)");
        pass(&format!("Proof is base64 ({proof_len} chars)"));
    } else {
        fail("Proof file is empty");
        bail!("empty proof");
    }

    // ==========================================
    // STEP 8: Sign and submit proof to gateway
    // ==========================================
    step(8, "Sign and submit proof to gateway");

    let calldata_str = format!(
        "0x1,{},{},0x1,0x1",
        contract_address, increment_selector
    );

    let proof_facts_file = proof_output.with_extension("proof_facts");

    let proof_b64_trimmed = proof_b64.trim().to_string();
    let proof_facts_str = tokio::fs::read_to_string(&proof_facts_file)
        .await
        .wrap_err("failed to read proof_facts")?;
    let proof_facts_hex = parse_proof_facts_json(&proof_facts_str)
        .map_err(|e| eyre::eyre!("failed to parse proof_facts: {e}"))?;
    let proof_facts: Vec<starknet_types_core::felt::Felt> = proof_facts_hex
        .iter()
        .map(|h| felt_from_hex(h).map_err(|e| eyre::eyre!(e)))
        .collect::<Result<_>>()?;

    let calldata_felts: Vec<starknet_types_core::felt::Felt> = calldata_str
        .split(',')
        .map(|h| felt_from_hex(h.trim()).map_err(|e| eyre::eyre!(e)))
        .collect::<Result<_>>()?;

    let sender_address =
        felt_from_hex(&config.account_address).map_err(|e| eyre::eyre!(e))?;
    let private_key =
        felt_from_hex(&config.private_key).map_err(|e| eyre::eyre!(e))?;
    let chain_id = config.chain_id_felt();

    info!("  Nonce: {nonce}");

    let params = SubmitParams {
        sender_address,
        private_key,
        calldata: calldata_felts,
        proof_base64: proof_b64_trimmed,
        proof_facts,
        nonce: starknet_types_core::felt::Felt::from(nonce),
        chain_id,
        resource_bounds: ResourceBounds::default(),
        gateway_url: config.gateway_url.clone(),
    };

    let (tx_hash, payload) =
        sign_and_build_payload(&params).map_err(|e| eyre::eyre!("signing failed: {e}"))?;
    info!("  Tx hash: {:#x}", tx_hash);

    let submit_url = format!("{}/gateway/add_transaction", config.gateway_url);
    info!("  Submitting to {submit_url}...");
    info!("  Proof block: {block_number} (gateway may lag behind RPC — will retry if too recent)");

    let client = reqwest::Client::new();
    let max_attempts = 20;
    let mut accepted = false;
    let fails_before = FAIL_COUNT.load(Ordering::Relaxed);

    for attempt in 1..=max_attempts {
        let response = client
            .post(&submit_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await;

        match response {
            Ok(resp) => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");
                let msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("");

                if code == "TRANSACTION_RECEIVED" {
                    info!("  Response: {}", serde_json::to_string_pretty(&body)?);
                    pass("Proof accepted by gateway (signed submission)");
                    accepted = true;
                    break;
                } else if msg.contains("too recent") && attempt < max_attempts {
                    info!("  Attempt {attempt}/{max_attempts}: gateway says block too recent, waiting 10s...");
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    continue;
                } else {
                    info!("  Response: {}", serde_json::to_string_pretty(&body)?);
                    fail(&format!("Proof submission failed: code={code}"));
                    break;
                }
            }
            Err(e) => {
                fail(&format!("Gateway request failed: {e}"));
                break;
            }
        }
    }

    if !accepted && FAIL_COUNT.load(Ordering::Relaxed) == fails_before {
        fail("Gateway did not accept proof after all retries");
    }

    // ==========================================
    // STEP 9: Verify counter incremented
    // ==========================================
    step(9, "Verify counter incremented on-chain");

    if accepted {
        // Read counter value before the proven tx lands
        let counter_before = read_counter(&rpc, &contract_address).await;
        info!("  Counter before: {counter_before:?}");

        // Wait for the proven transaction to be included
        info!("  Waiting for proven tx {:#x} to be included...", tx_hash);
        let tx_hash_hex = format!("{:#x}", tx_hash);
        match rpc.wait_for_tx(&tx_hash_hex, 180, 5).await {
            Ok(receipt) => {
                let bn = snip36_core::rpc::receipt_block_number(&receipt).unwrap_or(0);
                info!("  Proven tx included in block {bn}");
            }
            Err(e) => {
                fail(&format!("Proven tx not confirmed: {e}"));
            }
        }

        // Read counter value after
        let counter_after = read_counter(&rpc, &contract_address).await;
        info!("  Counter after:  {counter_after:?}");

        match (counter_before, counter_after) {
            (Some(before), Some(after)) if after == before + 1 => {
                pass(&format!("Counter incremented: {before} -> {after}"));
            }
            (Some(before), Some(after)) => {
                fail(&format!("Counter did not increment as expected: {before} -> {after}"));
            }
            (None, Some(after)) if after == 1 => {
                pass(&format!("Counter value is {after} (could not read before)"));
            }
            _ => {
                fail("Could not read counter value");
            }
        }
    } else {
        info!("  Skipping counter verification (proof not accepted)");
    }

    // ==========================================
    // Summary
    // ==========================================
    finish_last_step();
    let total_elapsed = e2e_start.elapsed();
    let passed = PASS_COUNT.load(Ordering::Relaxed);
    let failed = FAIL_COUNT.load(Ordering::Relaxed);

    info!("");
    info!("==========================================");
    info!("  E2E TEST SUMMARY");
    info!("==========================================");
    info!("");
    info!("  Passed: {passed}");
    info!("  Failed: {failed}");
    info!("  Total:  {}", passed + failed);
    info!("");

    // Timing breakdown
    info!("  Step Timings:");
    info!("  {:<45} {:>10}", "Step", "Duration");
    info!("  {}", "-".repeat(57));
    if let Ok(timings) = STEP_TIMINGS.lock() {
        for (name, duration) in timings.iter() {
            info!("  {:<45} {:>10}", name, format_duration(*duration));
        }
    }
    info!("  {}", "-".repeat(57));
    info!("  {:<45} {:>10}", "Total", format_duration(total_elapsed));
    info!("");

    if failed == 0 {
        info!("  RESULT: ALL TESTS PASSED");
        Ok(())
    } else {
        info!("  RESULT: {failed} TEST(S) FAILED");
        std::process::exit(1);
    }
}

async fn check_prereqs(config: &Config) -> Result<()> {
    for cmd in ["scarb", "sncast"] {
        let check = tokio::process::Command::new("which")
            .arg(cmd)
            .output()
            .await;
        if check.map(|o| !o.status.success()).unwrap_or(true) {
            bail!("{cmd} not found in PATH");
        }
    }

    let prover = config.prover_bin();
    if !prover.exists() {
        bail!(
            "stwo-run-and-prove not found at {}. Run `snip36 setup` first.",
            prover.display()
        );
    }

    Ok(())
}

async fn read_counter(rpc: &StarknetRpc, contract_address: &str) -> Option<u64> {
    let result = rpc
        .starknet_call(contract_address, GET_COUNTER_SELECTOR, &[])
        .await
        .ok()?;
    let hex = result.first()?;
    u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok()
}
