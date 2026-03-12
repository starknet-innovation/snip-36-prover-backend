use serde_json::Value;
use tracing::debug;

/// Starknet JSON-RPC client.
#[derive(Debug, Clone)]
pub struct StarknetRpc {
    url: String,
    client: reqwest::Client,
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON-RPC error: {0}")]
    JsonRpc(String),
    #[error("unexpected response: {0}")]
    Unexpected(String),
    #[error("transaction {tx_hash} not confirmed within {timeout}s")]
    TxTimeout { tx_hash: String, timeout: u64 },
    #[error("transaction rejected: {0}")]
    TxRejected(String),
    #[error("no new block after {block_number} within {timeout}s")]
    BlockTimeout { block_number: u64, timeout: u64 },
}

impl StarknetRpc {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Send a raw JSON-RPC request and return the full response.
    pub async fn call_raw(&self, payload: Value) -> Result<Value, RpcError> {
        let resp = self
            .client
            .post(&self.url)
            .json(&payload)
            .send()
            .await?
            .json::<Value>()
            .await?;
        Ok(resp)
    }

    /// Send a JSON-RPC request and return just the `result` field.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });
        let resp = self.call_raw(payload).await?;
        if let Some(error) = resp.get("error") {
            return Err(RpcError::JsonRpc(error.to_string()));
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| RpcError::Unexpected(resp.to_string()))
    }

    /// Get the current block number.
    pub async fn block_number(&self) -> Result<u64, RpcError> {
        let result = self.call("starknet_blockNumber", serde_json::json!({})).await?;
        result
            .as_u64()
            .ok_or_else(|| RpcError::Unexpected(format!("expected u64 block number: {result}")))
    }

    /// Get the chain ID.
    pub async fn chain_id(&self) -> Result<String, RpcError> {
        let result = self.call("starknet_chainId", serde_json::json!({})).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| RpcError::Unexpected(format!("expected string chain id: {result}")))
    }

    /// Get the nonce for a contract address at the latest block.
    pub async fn get_nonce(&self, address: &str) -> Result<u64, RpcError> {
        self.get_nonce_at_block(address, serde_json::json!("latest")).await
    }

    /// Get the nonce for a contract address at a specific block.
    pub async fn get_nonce_at_block(
        &self,
        address: &str,
        block_id: serde_json::Value,
    ) -> Result<u64, RpcError> {
        let result = self
            .call(
                "starknet_getNonce",
                serde_json::json!({
                    "block_id": block_id,
                    "contract_address": address,
                }),
            )
            .await?;
        let hex_str = result
            .as_str()
            .ok_or_else(|| RpcError::Unexpected(format!("expected hex nonce: {result}")))?;
        u64::from_str_radix(hex_str.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Unexpected(format!("invalid nonce hex '{hex_str}': {e}")))
    }

    /// Call a contract view function.
    pub async fn starknet_call(
        &self,
        contract_address: &str,
        selector: &str,
        calldata: &[&str],
    ) -> Result<Vec<String>, RpcError> {
        let result = self
            .call(
                "starknet_call",
                serde_json::json!({
                    "request": {
                        "contract_address": contract_address,
                        "entry_point_selector": selector,
                        "calldata": calldata,
                    },
                    "block_id": "latest",
                }),
            )
            .await?;
        let arr = result
            .as_array()
            .ok_or_else(|| RpcError::Unexpected(format!("expected array: {result}")))?;
        Ok(arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
    }

    /// Get a transaction by hash.
    pub async fn get_transaction(&self, tx_hash: &str) -> Result<Value, RpcError> {
        self.call(
            "starknet_getTransactionByHash",
            serde_json::json!({ "transaction_hash": tx_hash }),
        )
        .await
    }

    /// Get a transaction receipt.
    pub async fn get_receipt(&self, tx_hash: &str) -> Result<Value, RpcError> {
        self.call(
            "starknet_getTransactionReceipt",
            serde_json::json!({ "transaction_hash": tx_hash }),
        )
        .await
    }

    /// Get a class by hash.
    pub async fn get_class(&self, class_hash: &str) -> Result<Value, RpcError> {
        self.call(
            "starknet_getClass",
            serde_json::json!({
                "block_id": "latest",
                "class_hash": class_hash,
            }),
        )
        .await
    }

    /// Submit an invoke transaction via JSON-RPC.
    pub async fn add_invoke_transaction(&self, invoke_tx: Value) -> Result<String, RpcError> {
        let result = self
            .call(
                "starknet_addInvokeTransaction",
                serde_json::json!({ "invoke_transaction": invoke_tx }),
            )
            .await?;
        result
            .get("transaction_hash")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| RpcError::Unexpected(format!("no transaction_hash in: {result}")))
    }

    /// Poll for a transaction receipt until confirmed in a block.
    ///
    /// Returns the receipt JSON once `finality_status` is ACCEPTED_ON_L2/L1
    /// and `block_number` is present (block is closed).
    pub async fn wait_for_tx(
        &self,
        tx_hash: &str,
        timeout_secs: u64,
        poll_interval_secs: u64,
    ) -> Result<Value, RpcError> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(RpcError::TxTimeout {
                    tx_hash: tx_hash.to_string(),
                    timeout: timeout_secs,
                });
            }
            if let Ok(receipt) = self.get_receipt(tx_hash).await {
                let status = receipt
                    .get("finality_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let execution_status = receipt
                    .get("execution_status")
                    .and_then(|v| v.as_str());
                let has_block = receipt.get("block_number").is_some();

                if status == "REJECTED"
                    || matches!(execution_status, Some("REVERTED"))
                {
                    return Err(RpcError::TxRejected(receipt.to_string()));
                }
                if (status == "ACCEPTED_ON_L2" || status == "ACCEPTED_ON_L1")
                    && has_block
                    && (execution_status.is_none()
                        || matches!(execution_status, Some("SUCCEEDED")))
                {
                    return Ok(receipt);
                }
            }
            debug!(tx_hash, "waiting for tx confirmation...");
            tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
        }
    }

    /// Wait until the chain head advances past the given block number.
    pub async fn wait_for_block_after(
        &self,
        block_number: u64,
        timeout_secs: u64,
        poll_interval_secs: u64,
    ) -> Result<u64, RpcError> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(RpcError::BlockTimeout {
                    block_number,
                    timeout: timeout_secs,
                });
            }
            if let Ok(current) = self.block_number().await {
                if current > block_number {
                    return Ok(current);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(poll_interval_secs)).await;
        }
    }
}

/// Extract block number from a receipt, handling both hex strings and integers.
pub fn receipt_block_number(receipt: &Value) -> Option<u64> {
    let bn = receipt.get("block_number")?;
    if let Some(n) = bn.as_u64() {
        return Some(n);
    }
    if let Some(s) = bn.as_str() {
        return u64::from_str_radix(s.trim_start_matches("0x"), 16).ok();
    }
    None
}
