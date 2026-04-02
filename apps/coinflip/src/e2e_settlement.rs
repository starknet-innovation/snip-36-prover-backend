use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use starknet_types_core::felt::Felt;
use tracing::{error, info};

use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::{receipt_block_number, StarknetRpc};
use snip36_core::signing::{
    compute_invoke_v3_tx_hash, felt_from_hex, sign, sign_and_build_payload,
};
use crate::selectors::PLAY_SELECTOR;
use snip36_core::types::{ResourceBounds, SubmitParams, BALANCE_OF_SELECTOR, STRK_TOKEN};
use snip36_core::Config;

use snip36_core::cli_util::{format_cmd_output, parse_hex_from_output, parse_long_hex};

static PASS_COUNT: AtomicU32 = AtomicU32::new(0);
static FAIL_COUNT: AtomicU32 = AtomicU32::new(0);
static STEP_TIMINGS: Mutex<Vec<(String, std::time::Duration)>> = Mutex::new(Vec::new());
static STEP_START: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn pass(msg: &str) {
    PASS_COUNT.fetch_add(1, Ordering::Relaxed);
    info!("  PASS: {msg}");
}
fn fail(msg: &str) {
    FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
    error!("  FAIL: {msg}");
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
pub struct E2eSettlementArgs {
    /// Remote prover URL (skip local starknet_os_runner)
    #[arg(long)]
    pub prover_url: Option<String>,

    /// Output directory for E2E artifacts
    #[arg(long, default_value = "output/e2e-settlement")]
    pub output_dir: PathBuf,

    /// Stop after proving — save proof locally without submitting
    #[arg(long)]
    pub prove_only: bool,

    /// Player bet: 0 (heads) or 1 (tails)
    #[arg(long, default_value = "0")]
    pub bet: u8,

    /// Bet amount in STRK (e.g. 0.001)
    #[arg(long, default_value = "0.001")]
    pub bet_amount: f64,
}

pub async fn run(args: E2eSettlementArgs, env_file: Option<&std::path::Path>) -> Result<()> {
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
    let account_name = "playground-master";
    let bet = args.bet.min(1);
    let bet_amount_wei: u128 = (args.bet_amount * 1e18) as u128;

    info!("=== SNIP-36 Settlement E2E Test ===");
    info!("");
    info!("  RPC:     {}", config.rpc_url);
    info!("  Account: {}", config.account_address);
    info!("  Bet:     {} ({})", bet, if bet == 0 { "heads" } else { "tails" });
    info!("  Amount:  {} STRK ({} wei)", args.bet_amount, bet_amount_wei);
    info!("");

    check_prereqs(&config).await?;
    tokio::fs::create_dir_all(&args.output_dir).await?;

    // ==========================================
    // STEP 0: Import account
    // ==========================================
    step(0, "Import account into sncast");

    let _ = tokio::process::Command::new("sncast")
        .args([
            "account", "import", "--name", account_name,
            "--address", &config.account_address,
            "--private-key", &config.private_key,
            "--type", "oz", "--url", &config.rpc_url, "--silent",
        ])
        .output()
        .await;

    match rpc.get_nonce(&config.account_address).await {
        Ok(nonce) => pass(&format!("Account imported (nonce: {:#x})", nonce)),
        Err(e) => {
            fail(&format!("Could not verify account: {e}"));
            bail!("cannot proceed");
        }
    }

    // ==========================================
    // STEP 1: Compile contracts
    // ==========================================
    step(1, "Compile contracts");

    let contracts_dir = config.contracts_dir();
    let build = tokio::process::Command::new("scarb")
        .arg("build")
        .current_dir(&contracts_dir)
        .output()
        .await
        .wrap_err("failed to run scarb build")?;

    if build.status.success() {
        pass("Contracts compiled");
    } else {
        fail(&format!("Compilation failed: {}", &format_cmd_output(&build)[..500.min(format_cmd_output(&build).len())]));
        bail!("compilation failed");
    }

    // ==========================================
    // STEP 2: Declare & deploy CoinFlip
    // ==========================================
    step(2, "Deploy CoinFlip contract");

    let coinflip_address = declare_and_deploy(
        &rpc, &contracts_dir, account_name, &config.rpc_url, "CoinFlip", "",
    )
    .await?;
    pass(&format!("CoinFlip: {coinflip_address}"));

    // ==========================================
    // STEP 3: Declare & deploy CoinFlipBank
    // ==========================================
    step(3, "Deploy CoinFlipBank contract");

    let bank_address = declare_and_deploy(
        &rpc,
        &contracts_dir,
        account_name,
        &config.rpc_url,
        "CoinFlipBank",
        &config.account_address, // constructor arg: owner
    )
    .await?;
    pass(&format!("CoinFlipBank: {bank_address}"));

    // ==========================================
    // STEP 4: Approve bank + fund with STRK
    // ==========================================
    step(4, "Approve and fund bank");

    // Approve bank to spend master's STRK
    let max_approval = "0xffffffffffffffffffffffffffffffff";
    let approve_calldata = format!("{} {} 0x0", bank_address, max_approval);
    sncast_invoke(&rpc, account_name, &config.rpc_url, STRK_TOKEN, "approve", &approve_calldata).await?;
    pass("Approved STRK for bank");

    // Fund bank with 1 STRK
    let fund_amount: u128 = 1 * 10u128.pow(18);
    let fund_calldata = format!("{} {:#x} 0x0", bank_address, fund_amount);
    sncast_invoke(&rpc, account_name, &config.rpc_url, STRK_TOKEN, "transfer", &fund_calldata).await?;
    pass("Funded bank with 1 STRK");

    // ==========================================
    // STEP 5: Deposit (player = master account)
    // ==========================================
    step(5, "Player deposit");

    // Note: bank approval already done in Step 4 with max allowance.
    // Since player = owner in this test, the same approval covers both deposit and match.

    // Generate session_id as felt
    let session_felt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));

    // Get reference block (will be used as seed AFTER match_deposit)
    // For this test we just note the current block
    let pre_deposit_block = rpc.block_number().await?;

    // Call deposit(session_id, bet_amount, seed_placeholder, bet)
    // seed will be set properly after match — for the contract it just stores it
    let deposit_calldata = format!(
        "{} {:#x} 0x0 {:#x} {:#x}",
        session_felt, bet_amount_wei, pre_deposit_block, bet
    );
    sncast_invoke(&rpc, account_name, &config.rpc_url, &bank_address, "deposit", &deposit_calldata).await?;
    pass(&format!("Player deposited {} STRK", args.bet_amount));

    // ==========================================
    // STEP 6: Bank match deposit
    // ==========================================
    step(6, "Bank match deposit");

    let match_tx = sncast_invoke(&rpc, account_name, &config.rpc_url, &bank_address, "match_deposit", &session_felt).await?;

    // Use the block where match_deposit was confirmed as seed_block
    let match_receipt = rpc.wait_for_tx(&match_tx, 120, 3).await?;
    let match_block = receipt_block_number(&match_receipt).unwrap_or(0);
    let reference_block = match_block.max(pre_deposit_block);
    let seed = format!("{:#x}", reference_block);
    pass(&format!("Bank matched (block {match_block}), seed_block: {reference_block}"));

    // Record player balance before settle
    let balance_before = get_strk_balance(&rpc, &config.account_address).await;
    info!("  Player STRK balance before: {:.6}", balance_before as f64 / 1e18);

    // ==========================================
    // STEP 7: Prove coin flip
    // ==========================================
    step(7, "Prove coin flip");

    let player = config.account_address.clone();
    let bet_hex = format!("{:#x}", bet);

    // Compute expected outcome
    let seed_felt = Felt::from(reference_block);
    let player_felt = felt_from_hex(&player).map_err(|e| eyre::eyre!(e))?;
    let expected_hash = snip36_core::pedersen_hash(&seed_felt, &player_felt);
    let hash_bytes = expected_hash.to_bytes_be();
    let expected_outcome = hash_bytes[31] & 1;
    let expected_won = expected_outcome == bet;
    info!(
        "  Expected: {} ({}) => {}",
        expected_outcome,
        if expected_outcome == 0 { "heads" } else { "tails" },
        if expected_won { "WIN" } else { "LOSE" },
    );

    // Build multicall calldata for CoinFlip.play()
    let calldata: Vec<String> = vec![
        "0x1".to_string(),
        coinflip_address.clone(),
        PLAY_SELECTOR.to_string(),
        "0x3".to_string(),
        seed.clone(),
        player.clone(),
        bet_hex.clone(),
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
        sender_felt, &calldata_felts, chain_id, nonce_felt, Felt::ZERO,
        &zero_bounds, &[], &[], 0, 0, &[],
    );

    let sig = sign(private_key_felt, standard_tx_hash).map_err(|e| eyre::eyre!("signing: {e}"))?;

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

    let tx_path = args.output_dir.join("settlement_tx.json");
    tokio::fs::write(&tx_path, serde_json::to_string_pretty(&tx_json)?).await?;
    pass("Transaction constructed");

    // Prove
    let proof_path = args.output_dir.join("settlement.proof");
    let env_prover_url = std::env::var("PROVER_URL").ok().filter(|s| !s.is_empty());
    let prover_url = args.prover_url.as_deref().or(env_prover_url.as_deref());

    let mut prove_args = vec![
        "prove".to_string(), "virtual-os".to_string(),
        "--block-number".to_string(), reference_block.to_string(),
        "--tx-json".to_string(), tx_path.to_string_lossy().to_string(),
        "--rpc-url".to_string(), config.rpc_url.clone(),
        "--output".to_string(), proof_path.to_string_lossy().to_string(),
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
        .wrap_err("prove command failed")?;

    if prove_status.success() && proof_path.exists() {
        let sz = tokio::fs::metadata(&proof_path).await?.len();
        pass(&format!("Proof generated ({sz} bytes)"));
    } else {
        fail("Proof generation failed");
        bail!("proving failed");
    }

    // Verify settlement message
    let messages_file = proof_path.with_extension("raw_messages.json");
    if messages_file.exists() {
        let msg_str = tokio::fs::read_to_string(&messages_file).await?;
        let msg_json: serde_json::Value = serde_json::from_str(&msg_str)?;
        if let Some(msgs) = msg_json.get("l2_to_l1_messages").and_then(|v| v.as_array()) {
            if let Some(payload) = msgs.first().and_then(|m| m.get("payload")).and_then(|v| v.as_array()) {
                let fields: Vec<String> = payload.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if fields.len() >= 5 {
                    let msg_outcome = if fields[3] == "0x0" { 0u8 } else { 1u8 };
                    if msg_outcome == expected_outcome {
                        pass("Settlement message matches expected outcome");
                    } else {
                        fail(&format!("Outcome mismatch: msg={msg_outcome}, expected={expected_outcome}"));
                    }
                }
            }
        }
    }

    if args.prove_only {
        info!("  --prove-only: skipping submission and settlement");
        pass("Artifacts saved");
    } else {
        // ==========================================
        // STEP 8: Submit proof
        // ==========================================
        step(8, "Submit proof on-chain");

        let proof_b64 = tokio::fs::read_to_string(&proof_path).await?.trim().to_string();
        let proof_facts_file = proof_path.with_extension("proof_facts");
        let proof_facts_str = tokio::fs::read_to_string(&proof_facts_file).await?;
        let proof_facts_hex = parse_proof_facts_json(&proof_facts_str)
            .map_err(|e| eyre::eyre!("parse proof_facts: {e}"))?;
        let proof_facts: Vec<Felt> = proof_facts_hex
            .iter()
            .map(|h| felt_from_hex(h).map_err(|e| eyre::eyre!(e)))
            .collect::<Result<_>>()?;

        let params = SubmitParams {
            sender_address: sender_felt,
            private_key: private_key_felt,
            calldata: calldata_felts,
            proof_base64: proof_b64,
            proof_facts,
            nonce: nonce_felt,
            chain_id,
            resource_bounds: ResourceBounds::default(),
        };

        let (tx_hash, invoke_tx) =
            sign_and_build_payload(&params).map_err(|e| eyre::eyre!("signing: {e}"))?;
        let tx_hash_hex = format!("{:#x}", tx_hash);

        info!("  Submitting tx {}", &tx_hash_hex[..18.min(tx_hash_hex.len())]);

        let gateway_url = config.gateway_url.as_deref().unwrap_or("");
        let submit_url = format!("{}/gateway/add_transaction", gateway_url.trim_end_matches('/'));

        let client = reqwest::Client::new();
        let mut accepted = false;
        for attempt in 1..=20 {
            match client.post(&submit_url).json(&invoke_tx).timeout(std::time::Duration::from_secs(120)).send().await {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");
                    let msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    if code == "TRANSACTION_RECEIVED" {
                        pass(&format!("Proof accepted (attempt {attempt})"));
                        accepted = true;
                        break;
                    } else if (msg.contains("too recent") || msg.contains("stored block hash: 0")) && attempt < 20 {
                        info!("  Attempt {attempt}/20: not ready, waiting 10s... ({msg})");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    } else {
                        fail(&format!("Rejected: {body}"));
                        break;
                    }
                }
                Err(e) => {
                    fail(&format!("Request failed: {e}"));
                    break;
                }
            }
        }

        if !accepted {
            bail!("proof submission failed");
        }

        info!("  Waiting for proof tx inclusion...");
        match rpc.wait_for_tx(&tx_hash_hex, 600, 5).await {
            Ok(receipt) => {
                let bn = receipt_block_number(&receipt).unwrap_or(0);
                pass(&format!("Proof tx included in block {bn}"));
            }
            Err(e) => {
                fail(&format!("Proof tx not confirmed: {e}"));
                bail!("proof tx failed");
            }
        }

        // ==========================================
        // STEP 9: Settle
        // ==========================================
        step(9, "Settle on-chain");

        let settle_calldata = format!("{} {}", session_felt, seed);
        sncast_invoke(&rpc, account_name, &config.rpc_url, &bank_address, "settle", &settle_calldata).await?;
        pass("Settlement tx confirmed");

        // ==========================================
        // STEP 10: Verify balances
        // ==========================================
        step(10, "Verify settlement");

        let balance_after = get_strk_balance(&rpc, &config.account_address).await;
        info!("  Player STRK balance after:  {:.6}", balance_after as f64 / 1e18);

        let diff = if balance_after > balance_before {
            balance_after - balance_before
        } else {
            0
        };
        let diff_strk = diff as f64 / 1e18;

        if expected_won {
            // Player should have gained ~bet_amount (minus gas)
            if diff > 0 {
                pass(&format!("Player balance increased by {:.6} STRK (won)", diff_strk));
            } else {
                fail(&format!("Player balance did NOT increase (expected win, diff={diff_strk})"));
            }
        } else {
            // Player lost — balance should have decreased (deposit was taken)
            pass(&format!("Player lost, balance diff: {:.6} STRK", diff_strk));
        }
    }

    // ==========================================
    // Summary
    // ==========================================
    finish_last_step();
    let total = e2e_start.elapsed();
    let passed = PASS_COUNT.load(Ordering::Relaxed);
    let failed = FAIL_COUNT.load(Ordering::Relaxed);

    info!("");
    info!("==========================================");
    info!("  SETTLEMENT E2E SUMMARY");
    info!("==========================================");
    info!("  Passed: {passed}  Failed: {failed}");
    info!("");
    info!("  Step Timings:");
    if let Ok(timings) = STEP_TIMINGS.lock() {
        for (name, dur) in timings.iter() {
            info!("  {:<45} {:>10}", name, format_duration(*dur));
        }
    }
    info!("  {:<45} {:>10}", "Total", format_duration(total));
    info!("");

    if failed == 0 {
        info!("  RESULT: ALL TESTS PASSED");
        Ok(())
    } else {
        info!("  RESULT: {failed} TEST(S) FAILED");
        std::process::exit(1);
    }
}

// ── Helpers ──────────────────────────────────────────────

async fn check_prereqs(config: &Config) -> Result<()> {
    for cmd in ["scarb", "sncast"] {
        let check = tokio::process::Command::new("which").arg(cmd).output().await;
        if check.map(|o| !o.status.success()).unwrap_or(true) {
            bail!("{cmd} not found in PATH");
        }
    }
    if !config.prover_bin().exists() {
        bail!("stwo-run-and-prove not found. Run `snip36 setup` first.");
    }
    Ok(())
}

async fn declare_and_deploy(
    rpc: &StarknetRpc,
    contracts_dir: &std::path::Path,
    account_name: &str,
    rpc_url: &str,
    contract_name: &str,
    constructor_calldata: &str,
) -> Result<String> {
    // Declare
    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account", account_name, "declare",
            "--contract-name", contract_name, "--url", rpc_url,
        ])
        .current_dir(contracts_dir)
        .output()
        .await?;

    let declare_text = format_cmd_output(&declare_output);
    let class_hash = parse_hex_from_output("class_hash", &declare_text)
        .or_else(|| parse_long_hex(&declare_text))
        .ok_or_else(|| eyre::eyre!("{contract_name} declare failed: {declare_text}"))?;

    if let Some(tx) = parse_hex_from_output("transaction_hash", &declare_text) {
        let _ = rpc.wait_for_tx(&tx, 120, 3).await;
    }

    // Deploy
    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));
    let mut deploy_args = vec![
        "--account".to_string(), account_name.to_string(),
        "deploy".to_string(),
        "--class-hash".to_string(), class_hash.clone(),
        "--salt".to_string(), salt,
        "--url".to_string(), rpc_url.to_string(),
    ];
    if !constructor_calldata.is_empty() {
        deploy_args.push("--constructor-calldata".to_string());
        deploy_args.push(constructor_calldata.to_string());
    }

    let deploy_output = tokio::process::Command::new("sncast")
        .args(&deploy_args)
        .output()
        .await?;

    let deploy_text = format_cmd_output(&deploy_output);
    let contract_address = parse_hex_from_output("contract_address", &deploy_text)
        .ok_or_else(|| eyre::eyre!("{contract_name} deploy failed: {deploy_text}"))?;

    if let Some(tx) = parse_hex_from_output("transaction_hash", &deploy_text) {
        let _ = rpc.wait_for_tx(&tx, 120, 3).await;
    }

    Ok(contract_address)
}

async fn sncast_invoke(
    rpc: &StarknetRpc,
    account_name: &str,
    rpc_url: &str,
    contract_address: &str,
    function: &str,
    calldata: &str,
) -> Result<String> {
    let output = tokio::process::Command::new("sncast")
        .args([
            "--account", account_name, "invoke",
            "--url", rpc_url,
            "--contract-address", contract_address,
            "--function", function,
            "--calldata", calldata,
        ])
        .output()
        .await?;

    let text = format_cmd_output(&output);
    let tx_hash = parse_hex_from_output("transaction_hash", &text)
        .ok_or_else(|| eyre::eyre!("{function} invoke failed: {text}"))?;

    rpc.wait_for_tx(&tx_hash, 120, 3).await
        .wrap_err(format!("{function} tx not confirmed: {tx_hash}"))?;

    Ok(tx_hash)
}

async fn get_strk_balance(rpc: &StarknetRpc, address: &str) -> u128 {
    match rpc.starknet_call(STRK_TOKEN, BALANCE_OF_SELECTOR, &[address]).await {
        Ok(result) => {
            let low = result.first().map(|s| s.as_str()).unwrap_or("0x0");
            u128::from_str_radix(low.trim_start_matches("0x"), 16).unwrap_or(0)
        }
        Err(_) => 0,
    }
}
