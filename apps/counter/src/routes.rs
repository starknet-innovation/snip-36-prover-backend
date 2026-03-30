//! Counter-contract-specific route handlers.
//!
//! These handlers are tied to the bundled Counter example contract and are
//! **not** part of the generic SNIP-36 server API.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use serde::{Deserialize, Serialize};
use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::receipt_block_number;
use snip36_core::signing::{
    compute_invoke_v3_tx_hash, felt_from_hex, sign, sign_and_build_payload,
};
use snip36_core::types::{ResourceBound, ResourceBounds, SubmitParams, STRK_TOKEN};
use starknet_types_core::felt::Felt;
use tokio::io::AsyncBufReadExt;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

use snip36_server::AppState;

use snip36_server::routes::fund::{error_response, parse_hex};
use snip36_server::routes::prove_block::find_snip36_bin;

use crate::selectors::{GET_COUNTER_SELECTOR, INCREMENT_SELECTOR};

// ── Counter routes ─────────────────────────────────────────────

/// Build the Counter-specific API router.
pub fn counter_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/deploy-counter", post(deploy_counter))
        .route("/api/invoke", post(invoke_increment))
        .route("/api/read-counter", post(read_counter))
        .route("/api/submit-proof", post(submit_proof))
        .route("/api/prove-block/{session_id}", get(prove_block))
}

// ════════════════════════════════════════════════════════════════
// Invoke increment
// ════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct InvokeRequest {
    pub session_id: String,
    #[serde(default = "default_amount")]
    pub amount: u64,
    pub signature_r: String,
    pub signature_s: String,
    pub nonce: u64,
}

fn default_amount() -> u64 {
    1
}

#[derive(Serialize)]
pub struct InvokeResponse {
    pub tx_hash: String,
    pub block_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// POST /api/invoke
///
/// Submit a pre-signed increment() invoke transaction via the RPC node.
pub async fn invoke_increment(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InvokeRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session = state.get_session(&req.session_id);

    let contract_address = session
        .contract_address
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No counter contract deployed"))?;

    let account_address = session
        .account_address
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No account deployed"))?;

    // Build multicall calldata: [num_calls, to, selector, calldata_len, ...calldata]
    let calldata = vec![
        "0x1".to_string(),
        contract_address.to_string(),
        INCREMENT_SELECTOR.to_string(),
        "0x1".to_string(),
        format!("{:#x}", req.amount),
    ];

    let resource_bounds = playground_bounds();

    let invoke_tx = serde_json::json!({
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": account_address,
        "calldata": calldata,
        "nonce": format!("{:#x}", req.nonce),
        "resource_bounds": resource_bounds.to_rpc_json(),
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "signature": [req.signature_r, req.signature_s],
    });

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "starknet_addInvokeTransaction",
        "params": { "invoke_transaction": invoke_tx },
        "id": 1,
    });

    let result = state
        .rpc
        .call_raw(payload)
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    if let Some(error) = result.get("error") {
        return Err(error_response(StatusCode::BAD_REQUEST, &error.to_string()));
    }

    let tx_hash = result
        .get("result")
        .and_then(|r| r.get("transaction_hash"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Unexpected RPC response: {result}"),
            )
        })?
        .to_string();

    info!(tx_hash = %tx_hash, "Invoke tx submitted");

    {
        state.update_session_with(&req.session_id, |session| {
            session.last_invoke_tx = Some(tx_hash.clone());
        });
    }

    // Wait for tx inclusion so the prover can reference the correct block
    match state.rpc.wait_for_tx(&tx_hash, 120, 2).await {
        Ok(receipt) => {
            let bn = receipt_block_number(&receipt);
            info!(block_number = ?bn, "Invoke tx confirmed");
            if let Some(block) = bn {
                state.update_session_with(&req.session_id, |session| {
                    session.invoke_block = Some(block);
                });
            }
            Ok(Json(InvokeResponse {
                tx_hash,
                block_number: bn,
                warning: None,
            }))
        }
        Err(_) => Ok(Json(InvokeResponse {
            tx_hash,
            block_number: None,
            warning: Some("Tx submitted but not yet confirmed".to_string()),
        })),
    }
}

// ════════════════════════════════════════════════════════════════
// Read counter
// ════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct ReadCounterRequest {
    pub contract_address: String,
}

#[derive(Serialize)]
pub struct ReadCounterResponse {
    pub counter_value: u64,
}

/// POST /api/read-counter
///
/// Call get_counter() on the deployed contract.
pub async fn read_counter(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReadCounterRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let result = state
        .rpc
        .starknet_call(&req.contract_address, GET_COUNTER_SELECTOR, &[])
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let value = result
        .first()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);

    Ok(Json(ReadCounterResponse {
        counter_value: value,
    }))
}

// ════════════════════════════════════════════════════════════════
// Deploy counter
// ════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct DeployCounterRequest {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct DeployCounterResponse {
    pub class_hash: String,
    pub contract_address: String,
    pub tx_hash: String,
    pub block_number: Option<u64>,
}

/// POST /api/deploy-counter
///
/// Declare + deploy the Counter contract (funded by master account).
pub async fn deploy_counter(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeployCounterRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let cwd = state.config.contracts_dir();
    let _lock = state.sncast_lock.lock().await;

    // Declare
    let declare_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &state.config.sncast_account(),
            "declare",
            "--contract-name",
            "Counter",
            "--url",
            &state.config.rpc_url,
        ])
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let declare_stdout = String::from_utf8_lossy(&declare_output.stdout);
    let declare_stderr = String::from_utf8_lossy(&declare_output.stderr);
    let declare_combined = format!("{declare_stdout}\n{declare_stderr}");

    // Extract class hash (long hex string, 50+ chars)
    let class_hash = extract_long_hex(&declare_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Declare failed: {declare_combined}"),
        )
    })?;

    // Deploy with random salt
    let salt = format!("0x{}", hex::encode(rand::random::<[u8; 16]>()));

    let deploy_output = tokio::process::Command::new("sncast")
        .args([
            "--account",
            &state.config.sncast_account(),
            "deploy",
            "--class-hash",
            &class_hash,
            "--salt",
            &salt,
            "--url",
            &state.config.rpc_url,
        ])
        .output()
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deploy_stdout = String::from_utf8_lossy(&deploy_output.stdout);
    let deploy_stderr = String::from_utf8_lossy(&deploy_output.stderr);
    let deploy_combined = format!("{deploy_stdout}\n{deploy_stderr}");

    if !deploy_output.status.success() {
        return Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!(
                "Counter deploy failed (exit {}): {deploy_combined}",
                deploy_output.status
            ),
        ));
    }

    let contract_address = parse_hex("contract_address", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Deploy failed: {deploy_combined}"),
        )
    })?;

    let tx_hash = parse_hex("transaction_hash", &deploy_combined).ok_or_else(|| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Deploy succeeded but no transaction_hash in output: {deploy_combined}"),
        )
    })?;

    info!(tx_hash = %tx_hash, "Deploy counter tx submitted");
    let receipt = state
        .rpc
        .wait_for_tx(&tx_hash, 120, 2)
        .await
        .map_err(|e| error_response(StatusCode::GATEWAY_TIMEOUT, &e.to_string()))?;
    let block_number = receipt_block_number(&receipt);
    info!(block_number = ?block_number, "Deploy counter confirmed");
    if let Some(block) = block_number {
        let _ = state.rpc.wait_for_block_after(block, 120, 2).await;
    }

    state.update_session_with(&req.session_id, |session| {
        session.contract_address = Some(contract_address.clone());
        session.class_hash = Some(class_hash.clone());
        session.deploy_block = block_number;
        session.last_reference_block = block_number;
    });

    Ok(Json(DeployCounterResponse {
        class_hash,
        contract_address,
        tx_hash,
        block_number,
    }))
}

/// Extract a long hex string (50+ hex digits) from text -- used for class hashes.
fn extract_long_hex(text: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"0x[0-9a-fA-F]{50,}").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

// ════════════════════════════════════════════════════════════════
// Submit proof
// ════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct SubmitProofRequest {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct SubmitProofResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_tx_hash: Option<String>,
    pub output: String,
}

/// POST /api/submit-proof
///
/// Sign and submit the proof-bearing transaction via RPC.
pub async fn submit_proof(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitProofRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session = state.get_session(&req.session_id);

    let proof_file = session
        .proof_file
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No proof file available"))?;

    let proof_path = Path::new(proof_file);
    if !proof_path.exists() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No proof file available",
        ));
    }

    let contract_address = session
        .contract_address
        .as_deref()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No counter contract in session"))?;

    // Read proof (base64 string)
    let proof_base64 = tokio::fs::read_to_string(proof_file)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to read proof file: {e}"),
            )
        })?
        .trim()
        .to_string();

    // Read proof_facts
    let proof_facts_file = proof_file.replace(".proof", ".proof_facts");
    let proof_facts_json = tokio::fs::read_to_string(&proof_facts_file)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to read proof_facts file: {e}"),
            )
        })?;

    let proof_facts_hex: Vec<String> = serde_json::from_str(&proof_facts_json).map_err(|e| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Invalid proof_facts JSON: {e}"),
        )
    })?;

    let proof_facts: Vec<Felt> = proof_facts_hex
        .iter()
        .map(|h| felt_from_hex(h))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    // Build calldata for increment(1): [num_calls, to, selector, calldata_len, amount]
    let calldata = vec![
        Felt::ONE,
        felt_from_hex(contract_address)
            .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?,
        felt_from_hex(INCREMENT_SELECTOR)
            .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?,
        Felt::ONE,
        Felt::ONE,
    ];

    // Sign and submit as the configured master account (whose private key we have)
    let sender_address = felt_from_hex(&state.config.account_address)
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let private_key = felt_from_hex(&state.config.private_key)
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let chain_id = state
        .config
        .chain_id_felt()
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    // Get nonce for the master account
    let nonce = state
        .rpc
        .get_nonce(&state.config.account_address)
        .await
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let params = SubmitParams {
        sender_address,
        private_key,
        calldata,
        proof_base64,
        proof_facts,
        nonce: Felt::from(nonce),
        chain_id,
        resource_bounds: playground_bounds(),
    };

    let (local_tx_hash, invoke_tx) = sign_and_build_payload(&params)
        .map_err(|e| error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let local_tx_hash_hex = format!("{:#x}", local_tx_hash);
    info!(local_tx_hash = %local_tx_hash_hex, "Submitting proof via RPC");

    // Submit via starknet_addInvokeTransaction
    let rpc_tx_hash = state
        .rpc
        .add_invoke_transaction(invoke_tx)
        .await
        .map_err(|e| {
            error_response(
                StatusCode::BAD_GATEWAY,
                &format!("RPC submission failed: {e}"),
            )
        })?;

    info!(
        local_tx_hash = %local_tx_hash_hex,
        rpc_tx_hash = %rpc_tx_hash,
        "RPC accepted transaction"
    );

    Ok(Json(SubmitProofResponse {
        tx_hash: Some(rpc_tx_hash.clone()),
        local_tx_hash: Some(local_tx_hash_hex),
        output: format!("{{\"transaction_hash\":\"{rpc_tx_hash}\"}}"),
    }))
}

// ════════════════════════════════════════════════════════════════
// Prove block (SSE streaming)
// ════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub struct ProveBlockQuery {
    #[serde(default = "default_one_u64")]
    increment_amount: u64,
    #[serde(default = "default_one_u32")]
    increments_per_block: u32,
}

fn default_one_u64() -> u64 {
    1
}
fn default_one_u32() -> u32 {
    1
}

/// GET /api/prove-block/{session_id}?increment_amount=1&increments_per_block=1
///
/// Full SNIP-36 cycle: construct tx off-chain -> prove in virtual OS -> submit
/// via RPC -> wait for inclusion -> verify counter. Streams progress via SSE.
pub async fn prove_block(
    State(state): State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(params): Query<ProveBlockQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session = state.get_session(&session_id);
    let contract_address = session
        .contract_address
        .clone()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No counter contract deployed"))?;
    let reference_block = session
        .last_reference_block
        .or(session.deploy_block)
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "Deploy block not tracked"))?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    let state = Arc::clone(&state);
    let increment_amount = params.increment_amount;
    let increments_per_block = params.increments_per_block;

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

        let expected_increment = increment_amount * increments_per_block as u64;

        send(
            "log",
            &format!(
                "Starting SNIP-36 cycle: {} calls x increment({}) = +{} per block",
                increments_per_block, increment_amount, expected_increment
            ),
        )
        .await;

        // -- Phase 1: Construct transaction --
        send("phase", "constructing").await;

        // Build multicall calldata
        let mut calldata_strs: Vec<String> = vec![format!("{:#x}", increments_per_block)];
        for _ in 0..increments_per_block {
            calldata_strs.push(contract_address.clone());
            calldata_strs.push(INCREMENT_SELECTOR.to_string());
            calldata_strs.push("0x1".to_string());
            calldata_strs.push(format!("{:#x}", increment_amount));
        }

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
                send("error", &format!("Invalid account address: {e}")).await;
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
            Ok(id) => id,
            Err(e) => {
                send("error", &format!("Invalid chain_id: {e}")).await;
                return;
            }
        };
        let resource_bounds = ResourceBounds::default();

        // Get nonce at reference_block so the tx is valid for the state being proven,
        // rather than at `latest` which may have been bumped by other sessions/txs.
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
                send(
                    "error",
                    &format!("Failed to get nonce at block {reference_block}: {e}"),
                )
                .await;
                return;
            }
        };
        let nonce_felt = Felt::from(nonce);

        let standard_tx_hash = compute_invoke_v3_tx_hash(
            sender_felt,
            &calldata_felts,
            chain_id,
            nonce_felt,
            Felt::ZERO,
            &resource_bounds,
            &[],
            &[],
            0,
            0,
            &[], // no proof_facts for virtual OS validation
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
            "resource_bounds": resource_bounds.to_rpc_json(),
            "tip": "0x0",
            "paymaster_data": [],
            "account_deployment_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "signature": [format!("{:#x}", sig.r), format!("{:#x}", sig.s)],
        });

        let output_dir = state.config.output_dir.join("playground");
        if let Err(e) = tokio::fs::create_dir_all(&output_dir).await {
            send("error", &format!("Failed to create output dir: {e}")).await;
            return;
        }

        let tx_path = output_dir.join(format!("{session_id}_tx.json"));
        if let Err(e) =
            tokio::fs::write(&tx_path, serde_json::to_string_pretty(&tx_json).unwrap()).await
        {
            send("error", &format!("Failed to write tx JSON: {e}")).await;
            return;
        }

        send(
            "log",
            &format!("Transaction constructed (nonce: {nonce}, ref block: {reference_block})"),
        )
        .await;

        // -- Phase 2: Prove in virtual OS --
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

        // Check for PROVER_URL env var for remote prover
        if let Ok(prover_url) = std::env::var("PROVER_URL") {
            if !prover_url.is_empty() {
                prove_args.push("--prover-url".to_string());
                prove_args.push(prover_url);
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

        // Stream stdout and stderr as log events
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let tx_stdout = tx.clone();
        let stdout_handle = tokio::spawn(async move {
            if let Some(stdout) = stdout {
                let reader = tokio::io::BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.is_empty() {
                        let _ = tx_stdout
                            .send(Ok(Event::default().event("log").data(line)))
                            .await;
                    }
                }
            }
        });

        let tx_stderr = tx.clone();
        let stderr_handle = tokio::spawn(async move {
            if let Some(stderr) = stderr {
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.is_empty() {
                        let _ = tx_stderr
                            .send(Ok(Event::default().event("log").data(line)))
                            .await;
                    }
                }
            }
        });

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;
        let status = child.wait().await;

        if !status.map(|s| s.success()).unwrap_or(false) || !proof_path.exists() {
            send("error", "Proof generation failed").await;
            return;
        }

        let proof_size = tokio::fs::metadata(&proof_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        send("log", &format!("Proof generated ({} bytes)", proof_size)).await;

        // Check for L2->L1 messages
        let messages_file = proof_path.with_extension("raw_messages.json");
        if messages_file.exists() {
            send(
                "log",
                &format!("L2->L1 messages saved: {}", messages_file.display()),
            )
            .await;
        }

        // -- Phase 3: Submit via RPC --
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

        let (local_tx_hash, invoke_tx) = match sign_and_build_payload(&params) {
            Ok(r) => r,
            Err(e) => {
                send("error", &format!("SNIP-36 signing failed: {e}")).await;
                return;
            }
        };

        let local_tx_hash_hex = format!("{:#x}", local_tx_hash);

        send(
            "log",
            &format!(
                "Submitting tx {} via RPC...",
                local_tx_hash_hex.get(..18).unwrap_or(&local_tx_hash_hex)
            ),
        )
        .await;

        let max_attempts = 20;
        let mut rpc_tx_hash = None;

        for attempt in 1..=max_attempts {
            match state.rpc.add_invoke_transaction(invoke_tx.clone()).await {
                Ok(accepted_tx_hash) => {
                    send(
                        "log",
                        &format!(
                            "RPC accepted (attempt {attempt}/{max_attempts}): {accepted_tx_hash}"
                        ),
                    )
                    .await;
                    rpc_tx_hash = Some(accepted_tx_hash);
                    break;
                }
                Err(snip36_core::rpc::RpcError::JsonRpc(msg)) if attempt < max_attempts => {
                    send(
                        "log",
                        &format!(
                            "RPC error (attempt {attempt}/{max_attempts}), waiting 10s... ({msg})"
                        ),
                    )
                    .await;
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
                Err(e) => {
                    send("error", &format!("RPC submission failed: {e}")).await;
                    return;
                }
            }
        }

        let Some(rpc_tx_hash) = rpc_tx_hash else {
            send("error", "RPC did not accept after all retries").await;
            return;
        };

        // -- Phase 4: Wait for inclusion and verify --
        send("phase", "verifying").await;
        send("log", "Waiting for tx inclusion...").await;

        match state.rpc.wait_for_tx(&rpc_tx_hash, 180, 5).await {
            Ok(receipt) => {
                let bn = receipt_block_number(&receipt).unwrap_or(0);
                send("log", &format!("Tx included in block {bn}")).await;

                // Update reference block for next prove
                state.update_session_with(&session_id, |session| {
                    session.last_reference_block = Some(bn);
                });
            }
            Err(e) => {
                send("error", &format!("Tx not confirmed: {e}")).await;
                return;
            }
        }

        // Read counter
        let counter_value = match state
            .rpc
            .starknet_call(&contract_address, GET_COUNTER_SELECTOR, &[])
            .await
        {
            Ok(result) => result
                .first()
                .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0),
            Err(_) => 0,
        };

        send("log", &format!("Counter value: {counter_value}")).await;

        send_json(
            "complete",
            serde_json::json!({
                "tx_hash": rpc_tx_hash,
                "counter_value": counter_value,
                "proof_size": proof_size,
                "increment": expected_increment,
            }),
        )
        .await;
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream))
}

// ════════════════════════════════════════════════════════════════
// Shared helpers
// ════════════════════════════════════════════════════════════════

/// Resource bounds tuned for the playground web UI (lower l2_gas).
pub fn playground_bounds() -> ResourceBounds {
    const SEPOLIA_GAS_PRICE_CEIL: u128 = 0x38d7ea4c68000;
    ResourceBounds {
        l1_gas: ResourceBound {
            max_amount: 0x0,
            max_price_per_unit: SEPOLIA_GAS_PRICE_CEIL,
        },
        l2_gas: ResourceBound {
            max_amount: 0x2000000,
            max_price_per_unit: 0x2cb417800,
        },
        l1_data_gas: ResourceBound {
            max_amount: 0x1b0,
            max_price_per_unit: SEPOLIA_GAS_PRICE_CEIL,
        },
    }
}
