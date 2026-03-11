use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::Json;
use tokio::io::AsyncBufReadExt;
use tokio_stream::wrappers::ReceiverStream;

use snip36_core::types::STRK_TOKEN;

use crate::state::AppState;

use super::fund::error_response;

/// GET /api/prove/{session_id}
///
/// Run virtual OS + stwo prover. Returns SSE stream of log lines.
pub async fn prove_transaction(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session = state.get_session(&session_id);
    let tx_hash = session
        .last_invoke_tx
        .clone()
        .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "No invoke tx to prove"))?;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    let state_clone = Arc::clone(&state);
    let session_id_clone = session_id.clone();

    tokio::spawn(async move {
        let state = state_clone;
        let session_id = session_id_clone;

        let send = |event: &str, data: &str| {
            let tx = tx.clone();
            let event = event.to_string();
            let data = data.to_string();
            async move {
                let _ = tx.send(Ok(Event::default().event(event).data(data))).await;
            }
        };

        send(
            "log",
            &format!("Starting proof generation for {tx_hash}..."),
        )
        .await;

        // Get or wait for the invoke block number
        let mut invoke_block = {
            let s = state.get_session(&session_id);
            s.invoke_block
        };

        if invoke_block.is_none() {
            send("log", "Waiting for tx inclusion...").await;
            match state.rpc.wait_for_tx(&tx_hash, 120, 2).await {
                Ok(receipt) => {
                    invoke_block = snip36_core::rpc::receipt_block_number(&receipt);
                }
                Err(_) => {
                    send("error", "Tx not included in time").await;
                    return;
                }
            }
        }

        let invoke_block = match invoke_block {
            Some(b) => b,
            None => {
                send("error", "Could not determine block number").await;
                return;
            }
        };

        let prove_block = invoke_block - 1;
        send(
            "log",
            &format!("Tx included in block {invoke_block}. Proving against block {prove_block}..."),
        )
        .await;

        {
            let mut session = state.get_session(&session_id);
            session.prove_block = Some(prove_block);
            state.update_session(&session_id, session);
        }

        // Create output directory
        let output_dir = state.config.output_dir.join("playground");
        if let Err(e) = tokio::fs::create_dir_all(&output_dir).await {
            send("error", &format!("Failed to create output dir: {e}")).await;
            return;
        }

        let proof_output = output_dir.join(format!("{session_id}.proof"));
        let scripts_dir = state.config.project_dir.join("scripts");
        let run_script = scripts_dir.join("run-virtual-os.sh");

        let child = tokio::process::Command::new(&run_script)
            .args([
                "--block-number",
                &prove_block.to_string(),
                "--tx-hash",
                &tx_hash,
                "--rpc-url",
                &state.config.rpc_url,
                "--output",
                &proof_output.to_string_lossy(),
                "--strk-fee-token",
                STRK_TOKEN,
            ])
            .env("STARKNET_RPC_URL", &state.config.rpc_url)
            .env("STARKNET_ACCOUNT_ADDRESS", &state.config.account_address)
            .env("STARKNET_PRIVATE_KEY", &state.config.private_key)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                send("error", &format!("Failed to spawn prover: {e}")).await;
                return;
            }
        };

        // Merge stdout and stderr, stream lines as SSE
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
        let _ = child.wait().await;

        // Check for proof file
        if proof_output.exists() {
            let proof_size = tokio::fs::metadata(&proof_output)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            {
                let mut session = state.get_session(&session_id);
                session.proof_file = Some(proof_output.to_string_lossy().to_string());
                state.update_session(&session_id, session);
            }

            let data = serde_json::json!({
                "proof_size": proof_size,
                "proof_file": proof_output.to_string_lossy(),
            });
            let _ = tx
                .send(Ok(Event::default()
                    .event("complete")
                    .data(data.to_string())))
                .await;
        } else {
            let _ = tx
                .send(Ok(Event::default()
                    .event("error")
                    .data("Proof generation failed")))
                .await;
        }
    });

    let stream = ReceiverStream::new(rx);
    Ok(Sse::new(stream))
}
