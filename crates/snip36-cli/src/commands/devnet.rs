//! starknet-devnet lifecycle management for the e2e-devnet command.
//!
//! Spawns a local starknet-devnet instance with a deterministic seed, polls
//! for readiness, and tears it down when the handle is dropped.

use std::process::Stdio;
use std::time::{Duration, Instant};

use color_eyre::eyre::{bail, Result, WrapErr};
use serde_json::Value;
use tokio::process::{Child, Command};
use tracing::info;

/// Handle to a running starknet-devnet process.
///
/// Kills the child process on drop so the devnet doesn't outlive the test run.
pub struct DevnetHandle {
    child: Option<Child>,
    pub url: String,
}

impl DevnetHandle {
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Keep the devnet running after this handle is dropped.
    ///
    /// Use for --keep-alive flows; the child then lingers until the user kills it.
    pub fn detach(mut self) {
        if let Some(child) = self.child.take() {
            std::mem::forget(child);
        }
    }
}

impl Drop for DevnetHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // start_kill sends SIGKILL on Unix — devnet has no state to flush, so this is fine.
            let _ = child.start_kill();
        }
    }
}

/// Spawn starknet-devnet and wait until it responds to starknet_chainId.
///
/// `bin` — path to the `starknet-devnet` executable (or just "starknet-devnet" to resolve via PATH).
/// `port` — HTTP port (devnet default is 5050).
/// `seed` — deterministic account seed (0 gives stable addresses across runs).
pub async fn spawn(bin: &str, port: u16, seed: u64) -> Result<DevnetHandle> {
    let url = format!("http://127.0.0.1:{port}");

    info!("  Spawning starknet-devnet on port {port} (seed {seed})");

    let child = Command::new(bin)
        .args([
            "--port",
            &port.to_string(),
            "--seed",
            &seed.to_string(),
            "--accounts",
            "3",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .wrap_err_with(|| format!("failed to spawn {bin}; install with `cargo install starknet-devnet` or download a release binary"))?;

    let handle = DevnetHandle {
        child: Some(child),
        url: url.clone(),
    };

    wait_for_ready(&url, Duration::from_secs(30)).await?;
    info!("  starknet-devnet ready at {url}");

    Ok(handle)
}

/// Poll `starknet_chainId` until it succeeds or we hit `timeout`.
async fn wait_for_ready(url: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last_err: Option<String> = None;
    while Instant::now() < deadline {
        match rpc_call(url, "starknet_chainId", serde_json::json!([])).await {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(format!("{e}")),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!(
        "starknet-devnet did not become ready within {:?}: {}",
        timeout,
        last_err.as_deref().unwrap_or("unknown")
    );
}

/// First predeployed account from devnet (seed-deterministic).
#[derive(Debug, Clone)]
pub struct PredeployedAccount {
    pub address: String,
    pub private_key: String,
}

/// Fetch the first predeployed account via devnet's JSON-RPC `devnet_getPredeployedAccounts`.
pub async fn fetch_predeployed_account(url: &str) -> Result<PredeployedAccount> {
    let resp = rpc_call(url, "devnet_getPredeployedAccounts", serde_json::json!([])).await?;
    let first = resp
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| color_eyre::eyre::eyre!("devnet returned no predeployed accounts"))?;
    let address = first
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| color_eyre::eyre::eyre!("predeployed account missing address"))?
        .to_string();
    let private_key = first
        .get("private_key")
        .and_then(Value::as_str)
        .ok_or_else(|| color_eyre::eyre::eyre!("predeployed account missing private_key"))?
        .to_string();
    Ok(PredeployedAccount {
        address,
        private_key,
    })
}

/// Chain id reported by devnet, decoded from the short-string felt.
pub async fn fetch_chain_id(url: &str) -> Result<String> {
    let result = rpc_call(url, "starknet_chainId", serde_json::json!([])).await?;
    let hex = result
        .as_str()
        .ok_or_else(|| color_eyre::eyre::eyre!("starknet_chainId returned non-string: {result}"))?;
    Ok(decode_short_string(hex))
}

/// Tiny JSON-RPC helper. Returns the `result` field of the response, or an error
/// describing the HTTP/JSON-RPC failure.
async fn rpc_call(url: &str, method: &str, params: Value) -> Result<Value> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    let resp: Value = client
        .post(url)
        .json(&body)
        .send()
        .await
        .wrap_err_with(|| format!("failed to call {method}"))?
        .json()
        .await
        .wrap_err_with(|| format!("invalid JSON from {method}"))?;
    if let Some(err) = resp.get("error") {
        bail!("{method} returned JSON-RPC error: {err}");
    }
    resp.get("result")
        .cloned()
        .ok_or_else(|| color_eyre::eyre::eyre!("{method} returned no result: {resp}"))
}

/// Decode a hex-encoded Starknet short string (e.g. "0x534e5f5345504f4c4941" → "SN_SEPOLIA").
/// Returns the raw hex string if decoding fails so the caller can still set STARKNET_CHAIN_ID.
fn decode_short_string(hex: &str) -> String {
    let trimmed = hex.trim_start_matches("0x");
    let bytes = match hex::decode(trimmed) {
        Ok(b) => b,
        Err(_) => return hex.to_string(),
    };
    let trimmed_bytes: Vec<u8> = bytes.into_iter().skip_while(|&b| b == 0).collect();
    match String::from_utf8(trimmed_bytes) {
        Ok(s) if s.chars().all(|c| c.is_ascii_graphic() || c == '_') => s,
        _ => hex.to_string(),
    }
}
