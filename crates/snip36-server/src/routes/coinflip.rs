use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use tokio_stream::wrappers::ReceiverStream;

use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::receipt_block_number;
use snip36_core::signing::{compute_invoke_v3_tx_hash, felt_from_hex, sign, sign_and_build_payload};
use snip36_core::types::{ResourceBounds, SubmitParams, PLAY_SELECTOR, STRK_TOKEN};
use starknet_types_core::felt::Felt;
use tracing::info;

use crate::state::{AppState, BetCommitment, CoinFlipDeployment};

use super::fund::error_response;
use super::prove_block::find_snip36_bin;

// ── Status ───────────────────────────────────────────────

#[derive(Serialize)]
pub struct CoinFlipStatusResponse {
    pub deployed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_address: Option<String>,
}

/// GET /api/coinflip/status
pub async fn coinflip_status(
    State(state): State<Arc<AppState>>,
) -> Json<CoinFlipStatusResponse> {
    let lock = state.coinflip.read().await;
    match lock.as_ref() {
        Some(d) => Json(CoinFlipStatusResponse {
            deployed: true,
            contract_address: Some(d.contract_address.clone()),
        }),
        None => Json(CoinFlipStatusResponse {
            deployed: false,
            contract_address: None,
        }),
    }
}

// ── Deploy ───────────────────────────────────────────────

#[derive(Serialize)]
pub struct DeployCoinFlipResponse {
    pub contract_address: String,
    pub class_hash: String,
    pub deploy_block: u64,
}

/// POST /api/coinflip/deploy
///
/// Declare + deploy the CoinFlip contract (one-time, idempotent).
pub async fn deploy_coinflip(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Return existing deployment if already deployed
    {
        let lock = state.coinflip.read().await;
        if let Some(d) = lock.as_ref() {
            return Ok(Json(DeployCoinFlipResponse {
                contract_address: d.contract_address.clone(),
                class_hash: d.class_hash.clone(),
                deploy_block: d.deploy_block,
            }));
        }
    }

    let cwd = state.config.contracts_dir();
    let account_name = state.config.sncast_account();

    // Declare CoinFlip
    info!("Declaring CoinFlip contract...");
    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account", &account_name,
            "declare", "--contract-name", "CoinFlip",
            "--url", &state.config.rpc_url,
        ])
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let declare_combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&declare_output.stdout),
        String::from_utf8_lossy(&declare_output.stderr),
    );

    let class_hash = extract_long_hex(&declare_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("CoinFlip declare failed: {declare_combined}"),
        )
    })?;
    info!(class_hash = %class_hash, "CoinFlip declared");

    // Wait for declare tx if present
    if let Some(tx) = parse_hex("transaction_hash", &declare_combined) {
        let _ = state.rpc.wait_for_tx(&tx, 120, 3).await;
    }

    // Deploy with random salt
    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));
    let deploy_output = tokio::process::Command::new("sncast")
        .args([
            "--account", &account_name,
            "deploy", "--class-hash", &class_hash,
            "--salt", &salt,
            "--url", &state.config.rpc_url,
        ])
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deploy_combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&deploy_output.stdout),
        String::from_utf8_lossy(&deploy_output.stderr),
    );

    if !deploy_output.status.success() {
        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("CoinFlip deploy failed: {deploy_combined}"),
        ));
    }

    let contract_address = parse_hex("contract_address", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("No contract_address in deploy output: {deploy_combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("No transaction_hash in deploy output: {deploy_combined}"),
        )
    })?;

    info!(tx_hash = %tx_hash, contract_address = %contract_address, "CoinFlip deployed");

    let receipt = state.rpc.wait_for_tx(&tx_hash, 120, 3).await.map_err(|e| {
        error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string())
    })?;
    let deploy_block = receipt_block_number(&receipt).unwrap_or(0);

    let deployment = CoinFlipDeployment {
        contract_address: contract_address.clone(),
        class_hash: class_hash.clone(),
        deploy_block,
    };

    // Store deployment and persist to disk
    {
        let mut lock = state.coinflip.write().await;
        *lock = Some(deployment);
    }
    state.save_deployments().await;

    Ok(Json(DeployCoinFlipResponse {
        contract_address,
        class_hash,
        deploy_block,
    }))
}

// ── Commit ───────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CommitRequest {
    /// pedersen(bet, nonce) computed by the player
    pub commitment: String,
    /// Player's wallet address
    pub player: String,
}

#[derive(Serialize)]
pub struct CommitResponse {
    pub session_id: String,
    pub seed_block: u64,
}

/// POST /api/coinflip/commit
///
/// Player commits their bet before the seed is locked.
/// The seed_block is NOT locked here — it's locked in confirm_deposit after
/// match_deposit completes, so the on-chain nonce at the seed_block already
/// includes the match_deposit transaction.
pub async fn commit_bet(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommitRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let session_felt = format!("0x{}", session_id.replace('-', ""));

    state.commitments.insert(
        session_id.clone(),
        BetCommitment {
            commitment: req.commitment.clone(),
            seed_block: 0, // placeholder — set in confirm_deposit
            player: req.player.clone(),
            bet_amount: None,
            session_felt: session_felt.clone(),
        },
    );

    info!(
        session_id = %session_id,
        commitment = %req.commitment,
        "Bet committed (seed will be locked after deposit)"
    );

    Ok(Json(CommitResponse {
        session_id,
        seed_block: 0, // client doesn't need this yet
    }))
}

// ── Play (SSE) — reveal + execute ────────────────────────

#[derive(Deserialize)]
pub struct PlayQuery {
    player: String,
    /// The revealed bet (0 or 1)
    #[serde(default)]
    bet: u8,
    /// The random nonce used in the commitment
    #[serde(default)]
    nonce: String,
}

/// GET /api/coinflip/play/{session_id}?player=0x...&bet=0|1&nonce=0x...
///
/// Reveal phase: verify commitment, then execute the coin flip.
/// Streams progress via SSE.
pub async fn play_coinflip(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<PlayQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let deployment = {
        let lock = state.coinflip.read().await;
        lock.clone().ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "CoinFlip contract not deployed. Call POST /api/coinflip/deploy first.",
            )
        })?
    };

    // Retrieve and consume the commitment
    let commitment = state
        .commitments
        .remove(&session_id)
        .map(|(_, c)| c)
        .ok_or_else(|| {
            error_response(
                StatusCode::BAD_REQUEST,
                "No commitment found for this session. Call POST /api/coinflip/commit first.",
            )
        })?;

    let bet = params.bet.min(1);
    let player = params.player.clone();

    // Verify player matches
    if player != commitment.player {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "Player address does not match commitment.",
        ));
    }

    // Verify commitment: pedersen(bet, nonce) == commitment
    let bet_felt = Felt::from(bet as u64);
    let nonce_felt = felt_from_hex(&params.nonce).map_err(|e| {
        error_response(StatusCode::BAD_REQUEST, &format!("Invalid nonce: {e}"))
    })?;
    let computed = snip36_core::pedersen_hash(&bet_felt, &nonce_felt);
    let computed_hex = format!("{:#x}", computed);

    if computed_hex != commitment.commitment {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "Commitment mismatch: pedersen({bet}, nonce) = {computed_hex}, expected {}",
                commitment.commitment
            ),
        ));
    }

    // Use the seed_block locked at commit time
    let seed_block = commitment.seed_block;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    let state = Arc::clone(&state);

    tokio::spawn(async move {
        let send = |event: &str, data: &str| {
            let tx = tx.clone();
            let event = event.to_string();
            let data = data.to_string();
            async move {
                let _ = tx.send(Ok(Event::default().event(event).data(data))).await;
            }
        };

        let send_json = |event: &str, data: serde_json::Value| {
            let tx = tx.clone();
            let event = event.to_string();
            async move {
                let _ = tx
                    .send(Ok(Event::default().event(event).data(data.to_string())))
                    .await;
            }
        };

        let contract_address = &deployment.contract_address;
        let reference_block = seed_block;

        send(
            "log",
            &format!(
                "CoinFlip: player={} bet={} ({})",
                &player[..player.len().min(18)],
                bet,
                if bet == 0 { "heads" } else { "tails" },
            ),
        )
        .await;

        // ── Phase 1: Construct transaction ──────────────────
        send("phase", "constructing").await;

        let seed = format!("{:#x}", reference_block);
        let bet_hex = format!("{:#x}", bet);

        // Build calldata: 1 call to play(seed, player, bet)
        let calldata_strs: Vec<String> = vec![
            "0x1".to_string(),
            contract_address.clone(),
            PLAY_SELECTOR.to_string(),
            "0x3".to_string(),
            seed.clone(),
            player.clone(),
            bet_hex.clone(),
        ];

        let calldata_felts: Vec<Felt> = match calldata_strs
            .iter()
            .map(|h| felt_from_hex(h).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(f) => f,
            Err(e) => {
                send("error", &format!("Failed to parse calldata: {e}")).await;
                return;
            }
        };

        let sender_felt = match felt_from_hex(&state.config.account_address) {
            Ok(f) => f,
            Err(e) => {
                send("error", &format!("Invalid sender address: {e}")).await;
                return;
            }
        };
        let private_key_felt = match felt_from_hex(&state.config.private_key) {
            Ok(f) => f,
            Err(e) => {
                send("error", &format!("Invalid private key: {e}")).await;
                return;
            }
        };
        let chain_id = match state.config.chain_id_felt() {
            Ok(f) => f,
            Err(e) => {
                send("error", &format!("Invalid chain_id: {e}")).await;
                return;
            }
        };

        // Get nonce at reference block — the virtual OS requires this nonce.
        // match_deposit has already been confirmed BEFORE seed_block was locked,
        // so the nonce at seed_block already includes match_deposit.
        let nonce = match state
            .rpc
            .get_nonce_at_block(
                &state.config.account_address,
                serde_json::json!({"block_number": reference_block}),
            )
            .await
        {
            Ok(n) => n,
            Err(e) => {
                send("error", &format!("Failed to get nonce: {e}")).await;
                return;
            }
        };
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

        let sig = match sign(private_key_felt, standard_tx_hash) {
            Ok(s) => s,
            Err(e) => {
                send("error", &format!("Signing failed: {e}")).await;
                return;
            }
        };

        let tx_json = serde_json::json!({
            "type": "INVOKE",
            "version": "0x3",
            "sender_address": &state.config.account_address,
            "calldata": calldata_strs,
            "nonce": format!("{:#x}", nonce),
            "resource_bounds": zero_bounds.to_rpc_json(),
            "tip": "0x0",
            "paymaster_data": [],
            "account_deployment_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "signature": [format!("{:#x}", sig.r), format!("{:#x}", sig.s)],
        });

        let output_dir = state.config.output_dir.join("coinflip");
        let _ = tokio::fs::create_dir_all(&output_dir).await;
        let tx_path = output_dir.join(format!("{session_id}_tx.json"));
        if let Err(e) =
            tokio::fs::write(&tx_path, serde_json::to_string_pretty(&tx_json).unwrap()).await
        {
            send("error", &format!("Failed to write tx JSON: {e}")).await;
            return;
        }

        send(
            "log",
            &format!("Transaction constructed (nonce: {nonce}, seed: {seed})"),
        )
        .await;

        // ── Phase 2: Prove in virtual OS ────────────────────
        send("phase", "proving").await;
        send(
            "log",
            &format!("Proving against block {reference_block}..."),
        )
        .await;

        let proof_path = output_dir.join(format!("{session_id}.proof"));
        let snip36_bin = find_snip36_bin();

        let mut prove_args = vec![
            "prove".to_string(),
            "virtual-os".to_string(),
            "--block-number".to_string(),
            reference_block.to_string(),
            "--tx-json".to_string(),
            tx_path.to_string_lossy().to_string(),
            "--rpc-url".to_string(),
            state.config.rpc_url.clone(),
            "--output".to_string(),
            proof_path.to_string_lossy().to_string(),
            "--strk-fee-token".to_string(),
            STRK_TOKEN.to_string(),
        ];

        if let Ok(url) = std::env::var("PROVER_URL") {
            if !url.is_empty() {
                prove_args.push("--prover-url".to_string());
                prove_args.push(url);
            }
        }

        let child = tokio::process::Command::new(&snip36_bin)
            .args(&prove_args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                send(
                    "error",
                    &format!("Failed to spawn prover ({}): {e}", snip36_bin.display()),
                )
                .await;
                return;
            }
        };

        // Stream stdout/stderr
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(stdout) = stdout {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let reader = tokio::io::BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_clone
                        .send(Ok(Event::default().event("log").data(line)))
                        .await;
                }
            });
        }
        if let Some(stderr) = stderr {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_clone
                        .send(Ok(Event::default().event("log").data(line)))
                        .await;
                }
            });
        }

        let status = child.wait().await;
        if !status.map(|s| s.success()).unwrap_or(false) {
            send("error", "Proof generation failed").await;
            return;
        }

        let proof_size = tokio::fs::metadata(&proof_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        send("log", &format!("Proof generated ({proof_size} bytes)")).await;

        // ── Parse settlement message ────────────────────────
        let messages_file = proof_path.with_extension("raw_messages.json");
        let mut outcome_label = String::new();
        let mut won = false;

        if messages_file.exists() {
            if let Ok(msg_str) = tokio::fs::read_to_string(&messages_file).await {
                if let Ok(msg_json) = serde_json::from_str::<serde_json::Value>(&msg_str) {
                    if let Some(msgs) = msg_json.get("l2_to_l1_messages").and_then(|v| v.as_array())
                    {
                        if let Some(first) = msgs.first() {
                            if let Some(payload) =
                                first.get("payload").and_then(|v| v.as_array())
                            {
                                let fields: Vec<String> = payload
                                    .iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect();
                                // Payload: [player, seed, bet, outcome, won]
                                if fields.len() >= 5 {
                                    outcome_label = if fields[3] == "0x0" {
                                        "heads".to_string()
                                    } else {
                                        "tails".to_string()
                                    };
                                    won = fields[4] == "0x1";
                                    send(
                                        "log",
                                        &format!(
                                            "Settlement: outcome={outcome_label} won={won}"
                                        ),
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Phase 3: Submit via RPC ─────────────────────────
        send("phase", "submitting").await;

        let proof_b64 = match tokio::fs::read_to_string(&proof_path).await {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                send("error", &format!("Failed to read proof: {e}")).await;
                return;
            }
        };

        let proof_facts_file = proof_path.with_extension("proof_facts");
        let proof_facts_str = match tokio::fs::read_to_string(&proof_facts_file).await {
            Ok(s) => s,
            Err(e) => {
                send("error", &format!("Failed to read proof_facts: {e}")).await;
                return;
            }
        };

        let proof_facts_hex = match parse_proof_facts_json(&proof_facts_str) {
            Ok(f) => f,
            Err(e) => {
                send("error", &format!("Invalid proof_facts: {e}")).await;
                return;
            }
        };

        let proof_facts: Vec<Felt> = match proof_facts_hex
            .iter()
            .map(|h| felt_from_hex(h).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(f) => f,
            Err(e) => {
                send("error", &format!("Failed to parse proof_facts: {e}")).await;
                return;
            }
        };

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

        let (gw_tx_hash, payload) = match sign_and_build_payload(&params) {
            Ok(r) => r,
            Err(e) => {
                send("error", &format!("SNIP-36 signing failed: {e}")).await;
                return;
            }
        };

        let gw_tx_hash_hex = format!("{:#x}", gw_tx_hash);
        let submit_url = format!(
            "{}/gateway/add_transaction",
            state.config.gateway_url.as_deref().unwrap_or("").trim_end_matches('/')
        );

        send(
            "log",
            &format!(
                "Submitting tx {}...",
                gw_tx_hash_hex.get(..18).unwrap_or(&gw_tx_hash_hex)
            ),
        )
        .await;

        let client = reqwest::Client::new();
        let max_attempts = 20;
        let mut accepted = false;

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
                    let body: serde_json::Value = match resp.json().await {
                        Ok(b) => b,
                        Err(e) => {
                            send("error", &format!("Failed to read response: {e}")).await;
                            return;
                        }
                    };
                    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");
                    let msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("");

                    if code == "TRANSACTION_RECEIVED" {
                        send(
                            "log",
                            &format!("Accepted (attempt {attempt}/{max_attempts})"),
                        )
                        .await;
                        accepted = true;
                        break;
                    } else if (msg.contains("too recent")
                        || msg.contains("stored block hash: 0"))
                        && attempt < max_attempts
                    {
                        send(
                            "log",
                            &format!("Not ready (attempt {attempt}/{max_attempts}), waiting 10s..."),
                        )
                        .await;
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    } else {
                        send("error", &format!("Rejected: {body}")).await;
                        return;
                    }
                }
                Err(e) => {
                    send("error", &format!("Request failed: {e}")).await;
                    return;
                }
            }
        }

        if !accepted {
            send("error", "Not accepted after all retries").await;
            return;
        }

        // ── Phase 4: Wait for inclusion ─────────────────────
        send("phase", "verifying").await;
        send("log", "Waiting for tx inclusion...").await;

        match state.rpc.wait_for_tx(&gw_tx_hash_hex, 180, 5).await {
            Ok(receipt) => {
                let bn = receipt_block_number(&receipt).unwrap_or(0);
                send("log", &format!("Tx included in block {bn}")).await;
            }
            Err(e) => {
                send("error", &format!("Tx not confirmed: {e}")).await;
                return;
            }
        }

        // ── Phase 5: Settlement ─────────────────────────────
        let mut settle_tx_hash_hex = String::new();
        let bank_addr = {
            let lock = state.bank.read().await;
            lock.as_ref().map(|b| b.contract_address.clone())
        };
        if let Some(bank_address) = bank_addr {
            send("phase", "settling").await;
            send("log", "Settling on-chain...").await;

            let session_felt = format!("0x{}", session_id.replace('-', ""));

            match sncast_invoke(&state, &bank_address, "settle", &session_felt).await {
                Ok((_output, Some(tx))) => {
                    send("log", &format!("Settlement tx: {}", tx.get(..18).unwrap_or(&tx))).await;
                    send("log", "Settlement confirmed").await;
                    settle_tx_hash_hex = tx;
                }
                Ok((_output, None)) => {
                    send("log", "Settlement: no tx hash in output").await;
                }
                Err(e) => {
                    send("log", &format!("Settlement failed (non-critical): {e}")).await;
                }
            }
        }

        // ── Send result ─────────────────────────────────────
        send_json(
            "result",
            serde_json::json!({
                "outcome": outcome_label,
                "bet": if bet == 0 { "heads" } else { "tails" },
                "won": won,
                "tx_hash": gw_tx_hash_hex,
                "settle_tx_hash": settle_tx_hash_hex,
                "proof_size": proof_size,
                "seed": seed,
                "player": player,
            }),
        )
        .await;
    });

    Ok(Sse::new(ReceiverStream::new(rx)))
}

// ── Bank routes ─────────────────────────────────────────

#[derive(Serialize)]
pub struct BankStatusResponse {
    pub deployed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_address: Option<String>,
}

/// GET /api/coinflip/bank/status
pub async fn bank_status(State(state): State<Arc<AppState>>) -> Json<BankStatusResponse> {
    let lock = state.bank.read().await;
    match lock.as_ref() {
        Some(d) => Json(BankStatusResponse {
            deployed: true,
            contract_address: Some(d.contract_address.clone()),
        }),
        None => Json(BankStatusResponse {
            deployed: false,
            contract_address: None,
        }),
    }
}

#[derive(Serialize)]
pub struct DeployBankResponse {
    pub contract_address: String,
    pub class_hash: String,
    pub deploy_block: u64,
}

/// POST /api/coinflip/bank/deploy
///
/// Declare + deploy CoinFlipBank, approve STRK spending, fund bank with initial STRK.
pub async fn deploy_bank(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Return existing if already deployed
    {
        let lock = state.bank.read().await;
        if let Some(d) = lock.as_ref() {
            return Ok(Json(DeployBankResponse {
                contract_address: d.contract_address.clone(),
                class_hash: d.class_hash.clone(),
                deploy_block: d.deploy_block,
            }));
        }
    }

    let cwd = state.config.contracts_dir();
    let account_name = state.config.sncast_account();

    // Declare CoinFlipBank
    info!("Declaring CoinFlipBank contract...");
    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account", &account_name,
            "declare", "--contract-name", "CoinFlipBank",
            "--url", &state.config.rpc_url,
        ])
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let declare_combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&declare_output.stdout),
        String::from_utf8_lossy(&declare_output.stderr),
    );

    let class_hash = extract_long_hex(&declare_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("CoinFlipBank declare failed: {declare_combined}"),
        )
    })?;
    info!(class_hash = %class_hash, "CoinFlipBank declared");

    if let Some(tx) = parse_hex("transaction_hash", &declare_combined) {
        let _ = state.rpc.wait_for_tx(&tx, 120, 3).await;
    }

    // Deploy with constructor(owner=master_account)
    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));
    let deploy_output = tokio::process::Command::new("sncast")
        .args([
            "--account", &account_name,
            "deploy", "--class-hash", &class_hash,
            "--constructor-calldata", &state.config.account_address,
            "--salt", &salt,
            "--url", &state.config.rpc_url,
        ])
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deploy_combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&deploy_output.stdout),
        String::from_utf8_lossy(&deploy_output.stderr),
    );

    let contract_address = parse_hex("contract_address", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("CoinFlipBank deploy failed: {deploy_combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("No tx_hash in deploy output: {deploy_combined}"),
        )
    })?;

    info!(tx_hash = %tx_hash, contract_address = %contract_address, "CoinFlipBank deployed");

    let receipt = state.rpc.wait_for_tx(&tx_hash, 120, 3).await.map_err(|e| {
        error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string())
    })?;
    let deploy_block = receipt_block_number(&receipt).unwrap_or(0);

    // Approve bank to spend master's STRK (max u128 approval)
    let max_approval = "0xffffffffffffffffffffffffffffffff";
    let approve_calldata = format!("{} {} {}", contract_address, max_approval, "0x0");
    info!("Approving STRK for CoinFlipBank...");
    let (_approve_output, _approve_tx) = sncast_invoke(
        &state, STRK_TOKEN, "approve", &approve_calldata,
    )
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    // Pre-fund the bank: transfer 10 STRK from master to the contract
    let fund_amount: u128 = 10 * 10u128.pow(18); // 10 STRK in wei
    let fund_calldata = format!("{} {:#x} 0x0", contract_address, fund_amount);
    info!("Pre-funding bank with 10 STRK...");
    let (_fund_output, _fund_tx) = sncast_invoke(
        &state, STRK_TOKEN, "transfer", &fund_calldata,
    )
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    let deployment = crate::state::BankDeployment {
        contract_address: contract_address.clone(),
        class_hash: class_hash.clone(),
        deploy_block,
    };

    {
        let mut lock = state.bank.write().await;
        *lock = Some(deployment);
    }
    state.save_deployments().await;

    Ok(Json(DeployBankResponse {
        contract_address,
        class_hash,
        deploy_block,
    }))
}

// ── Deposit info ────────────────────────────────────────

#[derive(Deserialize)]
pub struct DepositInfoRequest {
    pub session_id: String,
    pub bet_amount: f64,
}

#[derive(Serialize)]
pub struct DepositInfoResponse {
    pub bank_address: String,
    pub strk_address: String,
    pub session_felt: String,
    pub bet_amount_low: String,
    pub bet_amount_high: String,
    pub seed: String,
    pub bet: String,
}

/// POST /api/coinflip/deposit-info
///
/// Returns calldata for the player's approve + deposit multicall.
pub async fn deposit_info(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DepositInfoRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let bank = {
        let lock = state.bank.read().await;
        lock.clone().ok_or_else(|| {
            error_response(StatusCode::BAD_REQUEST, "Bank not deployed")
        })?
    };

    let commitment = state.commitments.get(&req.session_id).ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "No commitment for this session")
    })?;

    let amount_wei = (req.bet_amount * 1e18) as u128;
    let amount_low = format!("{:#x}", amount_wei);
    let amount_high = "0x0".to_string();

    let session_felt = commitment.session_felt.clone();
    let seed = format!("{:#x}", commitment.seed_block);
    // bet is in the play query params, not yet known here — use "0x0" placeholder,
    // the actual bet will be passed by the frontend directly
    drop(commitment);

    // Store bet_amount on the commitment
    if let Some(mut entry) = state.commitments.get_mut(&req.session_id) {
        entry.bet_amount = Some(amount_low.clone());
    }

    Ok(Json(DepositInfoResponse {
        bank_address: bank.contract_address,
        strk_address: STRK_TOKEN.to_string(),
        session_felt,
        bet_amount_low: amount_low,
        bet_amount_high: amount_high,
        seed,
        bet: "0x0".to_string(), // frontend overrides with actual bet
    }))
}

// ── Confirm deposit ─────────────────────────────────────

#[derive(Deserialize)]
pub struct ConfirmDepositRequest {
    pub session_id: String,
    pub tx_hash: String,
}

#[derive(Serialize)]
pub struct ConfirmDepositResponse {
    pub matched: bool,
    pub match_tx_hash: String,
}

/// POST /api/coinflip/confirm-deposit
///
/// Waits for the player's deposit tx, then matches it from the bank.
pub async fn confirm_deposit(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ConfirmDepositRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Wait for player's deposit tx
    state.rpc.wait_for_tx(&req.tx_hash, 120, 3).await.map_err(|e| {
        error_response(StatusCode::GATEWAY_TIMEOUT, &format!("Deposit tx not confirmed: {e}"))
    })?;

    let bank = {
        let lock = state.bank.read().await;
        lock.clone().ok_or_else(|| {
            error_response(StatusCode::BAD_REQUEST, "Bank not deployed")
        })?
    };

    let commitment = state.commitments.get(&req.session_id).ok_or_else(|| {
        error_response(StatusCode::BAD_REQUEST, "No commitment for this session")
    })?;
    let session_felt = commitment.session_felt.clone();
    drop(commitment);

    // Server matches the deposit (sncast_invoke holds lock + waits for confirmation)
    let (_match_output, match_tx) = sncast_invoke(
        &state, &bank.contract_address, "match_deposit", &session_felt,
    )
    .await
    .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    let match_tx = match_tx.ok_or_else(|| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "Bank match: no tx hash in output")
    })?;

    // NOW lock the seed_block — after match_deposit is confirmed.
    // Wait for one more block so the nonce at seed_block includes match_deposit.
    let current_block = state.rpc.block_number().await.unwrap_or(0);
    let _ = state.rpc.wait_for_block_after(current_block, 60, 3).await;
    let new_block = state.rpc.block_number().await.unwrap_or(current_block + 1);
    let deploy_block = {
        let lock = state.coinflip.read().await;
        lock.as_ref().map(|d| d.deploy_block).unwrap_or(0)
    };
    // Use new_block - 1 as seed; match_deposit is guaranteed to be included
    let seed_block = new_block.saturating_sub(1).max(deploy_block);

    // Update the commitment with the real seed_block
    if let Some(mut entry) = state.commitments.get_mut(&req.session_id) {
        entry.seed_block = seed_block;
    }

    info!(session_id = %req.session_id, match_tx = %match_tx, seed_block = seed_block, "Bank matched deposit, seed locked");

    Ok(Json(ConfirmDepositResponse {
        matched: true,
        match_tx_hash: match_tx,
    }))
}

// ── Player balance ──────────────────────────────────────

#[derive(Serialize)]
pub struct BalanceResponse {
    pub balance: String,
    pub balance_wei: String,
}

/// GET /api/coinflip/balance/{address}
pub async fn player_balance(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let balance_of_selector = "0x2e4263afad30923c891518314c3c95dbe830a16874e8abc5777a9a20b54c76e";

    let result = state
        .rpc
        .starknet_call(STRK_TOKEN, balance_of_selector, &[&address])
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("balance_of failed: {e}"))
        })?;

    // Result is [low, high] as hex strings
    let low = result.first().map(|s| s.as_str()).unwrap_or("0x0");
    let wei = u128::from_str_radix(low.trim_start_matches("0x"), 16).unwrap_or(0);
    let balance_f = wei as f64 / 1e18;

    Ok(Json(BalanceResponse {
        balance: format!("{:.4}", balance_f),
        balance_wei: low.to_string(),
    }))
}

// ── Bank balance (winnings) ──────────────────────────────

#[derive(Serialize)]
pub struct BankBalanceResponse {
    pub winnings: String,
    pub winnings_wei: String,
    pub bank_address: String,
}

/// GET /api/coinflip/winnings/{address}
///
/// Returns the player's withdrawable balance in the CoinFlipBank contract.
pub async fn player_winnings(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let bank = {
        let lock = state.bank.read().await;
        lock.clone().ok_or_else(|| {
            error_response(StatusCode::BAD_REQUEST, "Bank not deployed")
        })?
    };

    // get_balance(player) selector (from compiled contract entry_points_by_type)
    let get_balance_selector = "0x39e11d48192e4333233c7eb19d10ad67c362bb28580c604d67884c85da39695";

    let result = state
        .rpc
        .starknet_call(&bank.contract_address, get_balance_selector, &[&address])
        .await
        .map_err(|e| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("get_balance failed: {e}"))
        })?;

    // u256 result: [low, high]
    let low = result.first().map(|s| s.as_str()).unwrap_or("0x0");
    let wei = u128::from_str_radix(low.trim_start_matches("0x"), 16).unwrap_or(0);
    let balance_f = wei as f64 / 1e18;

    Ok(Json(BankBalanceResponse {
        winnings: format!("{:.4}", balance_f),
        winnings_wei: low.to_string(),
        bank_address: bank.contract_address,
    }))
}

// ── Helpers ──────────────────────────────────────────────

/// Run `sncast invoke` while holding the sncast mutex to prevent nonce races.
/// Waits for tx confirmation before releasing the lock.
async fn sncast_invoke(
    state: &crate::state::AppState,
    contract_address: &str,
    function: &str,
    calldata: &str,
) -> Result<(std::process::Output, Option<String>), String> {
    let _lock = state.sncast_lock.lock().await;
    let account_name = state.config.sncast_account();

    tracing::info!(function = function, "sncast invoke (serialized)");

    let output = tokio::process::Command::new("sncast")
        .args([
            "--account", &account_name,
            "invoke",
            "--url", &state.config.rpc_url,
            "--contract-address", contract_address,
            "--function", function,
            "--calldata", calldata,
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Wait for tx confirmation before releasing lock so next invoke sees updated nonce
    let tx_hash = parse_hex("transaction_hash", &combined);
    if let Some(ref tx) = tx_hash {
        let _ = state.rpc.wait_for_tx(tx, 120, 3).await;
    }

    Ok((output, tx_hash))
}

fn extract_long_hex(text: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"0x[0-9a-fA-F]{50,}").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

fn parse_hex(key: &str, text: &str) -> Option<String> {
    let pattern = format!(r"(?i){}[:\s]+\s*(0x[0-9a-fA-F]+)", key.replace('_', "[_ ]"));
    let re = regex_lite::Regex::new(&pattern).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}
