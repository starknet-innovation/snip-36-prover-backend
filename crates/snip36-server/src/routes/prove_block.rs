use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio_stream::wrappers::ReceiverStream;

use snip36_core::proof::parse_proof_facts_json;
use snip36_core::rpc::receipt_block_number;
use snip36_core::signing::{
    compute_invoke_v3_tx_hash, felt_from_hex, sign, sign_and_build_payload,
};
use snip36_core::types::{ResourceBounds, SubmitParams, INCREMENT_SELECTOR, STRK_TOKEN};
use starknet_types_core::felt::Felt;

use crate::state::AppState;

use super::fund::error_response;

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

/// Find the snip36 CLI binary.
///
/// Search order: SNIP36_CLI_BIN env, sibling of current exe (covers both
/// `cargo run` and installed layouts since both workspace binaries land in
/// the same target/{debug,release} directory), then PATH fallback.
pub fn find_snip36_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("SNIP36_CLI_BIN") {
        return PathBuf::from(bin);
    }
    if let Ok(exe) = std::env::current_exe() {
        // When running via `cargo run -p snip36-server`, the exe sits in
        // target/{debug,release}/ — the CLI binary is built there too as
        // long as both crates are in the same workspace.
        if let Some(dir) = exe.parent() {
            let cli = dir.join("snip36");
            if cli.exists() {
                return cli;
            }
        }
    }
    // Also check the workspace target dirs relative to CARGO_MANIFEST_DIR
    // (covers `cargo run` when current_exe resolves to a different path).
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        for profile in &["release", "debug"] {
            let candidate = PathBuf::from(&manifest_dir)
                .join("../../target")
                .join(profile)
                .join("snip36");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from("snip36")
}

/// GET /api/prove-block/{session_id}?increment_amount=1&increments_per_block=1
///
/// Full SNIP-36 cycle: construct tx off-chain -> prove in virtual OS -> submit
/// via RPC -> wait for inclusion -> verify counter. Streams progress via SSE.
pub async fn prove_block(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
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

        // ── Phase 1: Construct transaction ──────────────────
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

        // Check for L2→L1 messages
        let messages_file = proof_path.with_extension("raw_messages.json");
        if messages_file.exists() {
            send(
                "log",
                &format!("L2→L1 messages saved: {}", messages_file.display()),
            )
            .await;
        }

        // ── Phase 3: Submit via RPC ──────────────────────
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

        // ── Phase 4: Wait for inclusion and verify ──────────
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
            .starknet_call(
                &contract_address,
                snip36_core::types::GET_COUNTER_SELECTOR,
                &[],
            )
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
