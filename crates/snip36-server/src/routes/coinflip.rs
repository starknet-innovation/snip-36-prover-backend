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

    // Store deployment
    {
        let mut lock = state.coinflip.write().await;
        *lock = Some(deployment);
    }

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
/// Returns the session_id and the seed_block that will be used.
pub async fn commit_bet(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommitRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // Lock the seed at the current block
    let seed_block = state
        .rpc
        .block_number()
        .await
        .map(|n| n.saturating_sub(1))
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to get block number: {e}"),
            )
        })?;

    let session_id = uuid::Uuid::new_v4().to_string();

    state.commitments.insert(
        session_id.clone(),
        BetCommitment {
            commitment: req.commitment.clone(),
            seed_block,
            player: req.player.clone(),
        },
    );

    info!(
        session_id = %session_id,
        seed_block = seed_block,
        commitment = %req.commitment,
        "Bet committed"
    );

    Ok(Json(CommitResponse {
        session_id,
        seed_block,
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

        // Get nonce at reference block
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
            gateway_url: state.config.gateway_url.clone(),
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
            state.config.gateway_url.trim_end_matches('/')
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

        // ── Send result ─────────────────────────────────────
        send_json(
            "result",
            serde_json::json!({
                "outcome": outcome_label,
                "bet": if bet == 0 { "heads" } else { "tails" },
                "won": won,
                "tx_hash": gw_tx_hash_hex,
                "proof_size": proof_size,
                "seed": seed,
                "player": player,
            }),
        )
        .await;
    });

    Ok(Sse::new(ReceiverStream::new(rx)))
}

// ── Helpers ──────────────────────────────────────────────

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
