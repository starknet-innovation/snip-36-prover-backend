use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use starknet_types_core::felt::Felt;
use tracing::{error, info};

use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::StarknetRpc;
use snip36_core::signing::{
    compute_invoke_v3_tx_hash, felt_from_hex, sign, sign_and_build_payload,
};
use snip36_core::types::{ResourceBounds, SubmitParams, PLAY_SELECTOR, STRK_TOKEN};
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
pub struct E2eCoinflipArgs {
    /// Remote prover URL (skip local starknet_os_runner)
    #[arg(long)]
    prover_url: Option<String>,

    /// Output directory for E2E artifacts
    #[arg(long, default_value = "output/e2e-coinflip")]
    output_dir: PathBuf,

    /// Stop after proving — save proof and artifacts locally without submitting
    #[arg(long)]
    prove_only: bool,

    /// Player bet: 0 (heads) or 1 (tails)
    #[arg(long, default_value = "0")]
    bet: u8,
}

pub async fn run(args: E2eCoinflipArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    let config = Config::from_env(env_file)?;

    PASS_COUNT.store(0, Ordering::Relaxed);
    FAIL_COUNT.store(0, Ordering::Relaxed);
    if let Ok(mut t) = STEP_TIMINGS.lock() {
        t.clear();
    }
    if let Ok(mut s) = STEP_START.lock() {
        *s = None;
    }
    let e2e_start = Instant::now();

    let rpc = StarknetRpc::new(&config.rpc_url);
    let account_name = "e2e-test-account-2";
    let bet = args.bet.min(1); // clamp to 0 or 1

    info!("=== SNIP-36 CoinFlip E2E Test ===");
    info!("");
    info!("  RPC:     {}", config.rpc_url);
    info!("  Account: {}", config.account_address);
    info!(
        "  Bet:     {} ({})",
        bet,
        if bet == 0 { "heads" } else { "tails" }
    );
    info!("");

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

    match rpc.get_nonce(&config.account_address).await {
        Ok(nonce) => pass(&format!("Account imported (nonce: {:#x})", nonce)),
        Err(e) => {
            fail(&format!("Could not verify account on-chain: {e}"));
            bail!("cannot proceed without account");
        }
    }

    // ==========================================
    // STEP 1: Compile contracts
    // ==========================================
    step(1, "Compile CoinFlip contract");

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
        fail(&format!(
            "Compilation failed: {}",
            &out[..out.len().min(500)]
        ));
        bail!("compilation failed");
    }

    // ==========================================
    // STEP 2: Declare CoinFlip class
    // ==========================================
    step(2, "Declare CoinFlip class");

    let class_hash_output = tokio::process::Command::new("sncast")
        .args(["utils", "class-hash", "--contract-name", "CoinFlip"])
        .current_dir(&contracts_dir)
        .output()
        .await
        .wrap_err("failed to compute class hash")?;

    let class_hash_text = format_cmd_output(&class_hash_output);
    let computed_hash = parse_hex_from_output("class_hash", &class_hash_text)
        .or_else(|| parse_long_hex(&class_hash_text));

    let already_declared = if let Some(ref ch) = computed_hash {
        rpc.get_class(ch).await.is_ok()
    } else {
        false
    };

    let class_hash = if already_declared {
        let h = computed_hash.unwrap();
        pass("CoinFlip already declared");
        info!("  Class hash: {h}");
        h
    } else {
        let declare_output = tokio::process::Command::new("sncast")
            .args([
                "--account",
                account_name,
                "declare",
                "--url",
                &config.rpc_url,
                "--contract-name",
                "CoinFlip",
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

        let declare_tx = parse_hex_from_output("transaction_hash", &declare_combined);

        match class_hash {
            Some(h) => {
                pass("CoinFlip declared");
                info!("  Class hash: {h}");
                // Wait for declare tx to land before deploying
                if let Some(tx) = &declare_tx {
                    info!("  Waiting for declare tx inclusion...");
                    rpc.wait_for_tx(tx, 120, 3)
                        .await
                        .wrap_err("declare tx not confirmed")?;
                }
                h
            }
            None => {
                fail("Could not determine class hash");
                bail!("declare failed");
            }
        }
    };

    // ==========================================
    // STEP 3: Deploy CoinFlip
    // ==========================================
    step(3, "Deploy CoinFlip contract");

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
            pass("CoinFlip deployed");
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

    let reference_block = if let Some(tx) = &deploy_tx_hash {
        match rpc.wait_for_tx(tx, 120, 3).await {
            Ok(receipt) => {
                let bn = snip36_core::rpc::receipt_block_number(&receipt).unwrap_or(0);
                pass(&format!("Deploy confirmed in block {bn}"));
                bn
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
    // STEP 5: Play a round (prove the coin flip)
    // ==========================================
    step(5, "Prove coin flip");

    // Use reference block number as seed (public, deterministic)
    let seed = format!("{:#x}", reference_block);
    let player = config.account_address.clone();
    let bet_hex = format!("{:#x}", bet);

    info!("  Game parameters:");
    info!("    seed (block#): {seed}");
    info!("    player:        {player}");
    info!(
        "    bet:           {bet_hex} ({})",
        if bet == 0 { "heads" } else { "tails" }
    );

    // Compute expected outcome client-side for verification later
    let seed_felt = Felt::from(reference_block);
    let player_felt = felt_from_hex(&player).map_err(|e| eyre::eyre!(e))?;
    let expected_hash = snip36_core::pedersen_hash(&seed_felt, &player_felt);
    let hash_bytes = expected_hash.to_bytes_be();
    let expected_outcome = hash_bytes[31] & 1; // LSB
    let expected_won = if expected_outcome == bet { 1u8 } else { 0u8 };
    info!(
        "  Expected outcome: {} ({}) => {}",
        expected_outcome,
        if expected_outcome == 0 {
            "heads"
        } else {
            "tails"
        },
        if expected_won == 1 { "WIN" } else { "LOSE" },
    );

    // Build multicall calldata: 1 call to play(seed, player, bet)
    let calldata: Vec<String> = vec![
        "0x1".to_string(),         // 1 call
        contract_address.clone(),  // target
        PLAY_SELECTOR.to_string(), // selector
        "0x3".to_string(),         // calldata length (3 felts)
        seed.clone(),              // seed
        player.clone(),            // player
        bet_hex.clone(),           // bet
    ];

    let calldata_felts: Vec<Felt> = calldata
        .iter()
        .map(|h| felt_from_hex(h).map_err(|e| eyre::eyre!(e)))
        .collect::<Result<_>>()?;

    let sender_felt = felt_from_hex(&config.account_address).map_err(|e| eyre::eyre!(e))?;
    let private_key_felt = felt_from_hex(&config.private_key).map_err(|e| eyre::eyre!(e))?;
    let chain_id = config.chain_id_felt()?;

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let nonce = rpc.get_nonce(&config.account_address).await?;
    let nonce_felt = Felt::from(nonce);

    let zero_bounds = ResourceBounds::zero_fee();
    let standard_tx_hash = compute_invoke_v3_tx_hash(
        sender_felt,
        &calldata_felts,
        chain_id,
        nonce_felt,
        Felt::ZERO,
        &zero_bounds,
        &[],
        &[],
        0,
        0,
        &[],
    );

    let sig =
        sign(private_key_felt, standard_tx_hash).map_err(|e| eyre::eyre!("signing failed: {e}"))?;

    let tx_json = serde_json::json!({
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": &config.account_address,
        "calldata": calldata,
        "nonce": format!("{:#x}", nonce),
        "resource_bounds": zero_bounds.to_rpc_json(),
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "signature": [format!("{:#x}", sig.r), format!("{:#x}", sig.s)],
    });

    let tx_path = args.output_dir.join("coinflip_tx.json");
    tokio::fs::write(&tx_path, serde_json::to_string_pretty(&tx_json)?).await?;
    pass("Transaction constructed and signed");

    // --- Prove in virtual OS ---
    let proof_path = args.output_dir.join("coinflip.proof");

    let env_prover_url = std::env::var("PROVER_URL").ok().filter(|s| !s.is_empty());
    let prover_url = args.prover_url.as_deref().or(env_prover_url.as_deref());

    let mut prove_args = vec![
        "prove".to_string(),
        "virtual-os".to_string(),
        "--block-number".to_string(),
        reference_block.to_string(),
        "--tx-json".to_string(),
        tx_path.to_string_lossy().to_string(),
        "--rpc-url".to_string(),
        config.rpc_url.clone(),
        "--output".to_string(),
        proof_path.to_string_lossy().to_string(),
    ];

    if let Some(url) = prover_url {
        prove_args.push("--prover-url".to_string());
        prove_args.push(url.to_string());
    } else {
        prove_args.push("--strk-fee-token".to_string());
        prove_args.push(STRK_TOKEN.to_string());
    }

    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("snip36"));
    let prove_status = tokio::process::Command::new(&current_exe)
        .args(&prove_args)
        .status()
        .await
        .wrap_err("failed to run prove command")?;

    if prove_status.success() && proof_path.exists() {
        let metadata = tokio::fs::metadata(&proof_path).await?;
        pass(&format!("Proof generated ({} bytes)", metadata.len()));
    } else {
        fail("Proof generation failed");
        bail!("proving failed");
    }

    // ==========================================
    // STEP 6: Verify settlement message
    // ==========================================
    step(6, "Verify settlement message");

    let messages_file = proof_path.with_extension("raw_messages.json");
    if !messages_file.exists() {
        fail("raw_messages.json not found");
        bail!("missing raw_messages.json");
    }

    let messages_str = tokio::fs::read_to_string(&messages_file)
        .await
        .wrap_err("failed to read raw_messages.json")?;
    let messages_json: serde_json::Value =
        serde_json::from_str(&messages_str).wrap_err("invalid JSON in raw_messages.json")?;

    let l2_to_l1 = messages_json
        .get("l2_to_l1_messages")
        .and_then(|v| v.as_array());

    match l2_to_l1 {
        Some(msgs) if !msgs.is_empty() => {
            let msg = &msgs[0];
            let payload = msg.get("payload").and_then(|v| v.as_array());

            if let Some(p) = payload {
                let fields: Vec<String> = p
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();

                // Payload: [player, seed, bet, outcome, won]
                if fields.len() >= 5 {
                    info!("  Settlement receipt:");
                    info!("    player:  {}", fields[0]);
                    info!("    seed:    {}", fields[1]);
                    info!(
                        "    bet:     {} ({})",
                        fields[2],
                        if fields[2] == "0x0" { "heads" } else { "tails" }
                    );
                    info!(
                        "    outcome: {} ({})",
                        fields[3],
                        if fields[3] == "0x0" { "heads" } else { "tails" }
                    );
                    info!(
                        "    won:     {} ({})",
                        fields[4],
                        if fields[4] == "0x1" { "WIN" } else { "LOSE" }
                    );

                    // Verify outcome matches client-side computation
                    let msg_outcome = if fields[3] == "0x0" { 0u8 } else { 1u8 };
                    let msg_won = if fields[4] == "0x1" { 1u8 } else { 0u8 };

                    if msg_outcome == expected_outcome {
                        pass("Outcome matches client-side Poseidon hash");
                    } else {
                        fail(&format!(
                            "Outcome mismatch: contract says {msg_outcome}, expected {expected_outcome}"
                        ));
                    }

                    if msg_won == expected_won {
                        pass(&format!(
                            "Game result verified: {}",
                            if msg_won == 1 {
                                "PLAYER WINS"
                            } else {
                                "HOUSE WINS"
                            },
                        ));
                    } else {
                        fail("Win/loss mismatch");
                    }
                } else {
                    fail(&format!("Unexpected payload length: {}", fields.len()));
                }
            } else {
                fail("No payload in settlement message");
            }
        }
        _ => {
            fail("No L2→L1 messages in output");
            bail!("empty messages");
        }
    }

    // --- If prove-only, skip submission ---
    if args.prove_only {
        let proof_facts_file = proof_path.with_extension("proof_facts");
        info!("  --prove-only: skipping RPC submission");
        info!("  Proof:       {}", proof_path.display());
        info!("  Proof facts: {}", proof_facts_file.display());
        info!("  Messages:    {}", messages_file.display());
        pass("All artifacts saved locally");
    } else {
        // --- Submit via RPC ---
        step(7, "Submit via RPC");

        let proof_b64 = tokio::fs::read_to_string(&proof_path).await?;
        if proof_b64.trim().is_empty() {
            fail("Proof file is empty");
            bail!("empty proof");
        }

        let proof_facts_file = proof_path.with_extension("proof_facts");
        let proof_b64_trimmed = proof_b64.trim().to_string();
        let proof_facts_str = tokio::fs::read_to_string(&proof_facts_file)
            .await
            .wrap_err("failed to read proof_facts")?;
        let proof_facts_hex = parse_proof_facts_json(&proof_facts_str)
            .map_err(|e| eyre::eyre!("failed to parse proof_facts: {e}"))?;
        let proof_facts: Vec<Felt> = proof_facts_hex
            .iter()
            .map(|h| felt_from_hex(h).map_err(|e| eyre::eyre!(e)))
            .collect::<Result<_>>()?;

        let params = SubmitParams {
            sender_address: sender_felt,
            private_key: private_key_felt,
            calldata: calldata_felts,
            proof_base64: proof_b64_trimmed,
            proof_facts,
            nonce: nonce_felt,
            chain_id,
            resource_bounds: ResourceBounds::default(),
        };

        let (local_tx_hash, invoke_tx) =
            sign_and_build_payload(&params).map_err(|e| eyre::eyre!("signing failed: {e}"))?;
        let local_tx_hash_hex = format!("{:#x}", local_tx_hash);

        info!("  Submitting tx {local_tx_hash_hex} via RPC...");

        let max_attempts = 20;
        let mut rpc_tx_hash = None;

        for attempt in 1..=max_attempts {
            match rpc.add_invoke_transaction(invoke_tx.clone()).await {
                Ok(accepted_tx_hash) => {
                    pass(&format!(
                        "RPC accepted (attempt {attempt}/{max_attempts}): {accepted_tx_hash}"
                    ));
                    rpc_tx_hash = Some(accepted_tx_hash);
                    break;
                }
                Err(snip36_core::rpc::RpcError::JsonRpc(msg)) if attempt < max_attempts => {
                    info!("  Attempt {attempt}/{max_attempts}: RPC error, waiting 10s... ({msg})");
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
                Err(e) => {
                    fail(&format!("RPC submission failed: {e}"));
                    break;
                }
            }
        }

        let Some(rpc_tx_hash) = rpc_tx_hash else {
            bail!("RPC submission failed");
        };

        info!("  Waiting for tx {rpc_tx_hash} to be included...");

        match rpc.wait_for_tx(&rpc_tx_hash, 180, 5).await {
            Ok(receipt) => {
                let bn = snip36_core::rpc::receipt_block_number(&receipt).unwrap_or(0);
                pass(&format!("Tx included in block {bn}"));
            }
            Err(e) => {
                fail(&format!("Tx not confirmed: {e}"));
                bail!("tx confirmation failed");
            }
        }
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
    info!("  COINFLIP E2E SUMMARY");
    info!("==========================================");
    info!("");
    info!("  Passed: {passed}");
    info!("  Failed: {failed}");
    info!("  Total:  {}", passed + failed);
    info!("");

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
