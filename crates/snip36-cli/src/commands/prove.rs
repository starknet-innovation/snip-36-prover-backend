use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Subcommand};
use color_eyre::eyre::{bail, Result, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, error, info};

use snip36_core::Config;

#[derive(Args)]
pub struct ProveArgs {
    #[command(subcommand)]
    pub mode: ProveMode,
}

#[derive(Subcommand)]
pub enum ProveMode {
    /// Run starknet_os_runner + prover for a transaction
    VirtualOs {
        /// Block number to prove against
        #[arg(long)]
        block_number: u64,

        /// Transaction hash to fetch from RPC and prove
        #[arg(long, required_unless_present = "tx_json")]
        tx_hash: Option<String>,

        /// Path to a JSON file containing the transaction object (alternative to --tx-hash)
        #[arg(long, conflicts_with = "tx_hash")]
        tx_json: Option<PathBuf>,

        /// Starknet RPC endpoint URL
        #[arg(long)]
        rpc_url: String,

        /// Output proof path
        #[arg(long, default_value = "output/virtual_os.proof")]
        output: PathBuf,

        /// Remote prover URL (skip local server startup)
        #[arg(long)]
        prover_url: Option<String>,

        /// Port for local runner server
        #[arg(long, default_value_t = 9900)]
        port: u16,

        /// Override STRK fee token address
        #[arg(long)]
        strk_fee_token: Option<String>,
    },

    /// Prove a compiled Cairo program directly
    Program {
        /// Path to compiled Cairo program (JSON)
        #[arg(long)]
        program: PathBuf,

        /// Output path for the proof
        #[arg(long)]
        output: PathBuf,

        /// Program input (JSON)
        #[arg(long)]
        input: Option<PathBuf>,

        /// Prover parameters JSON
        #[arg(long)]
        params: Option<PathBuf>,

        /// Verify the proof after generation
        #[arg(long)]
        verify: bool,
    },

    /// Prove a Cairo PIE via bootloader
    Pie {
        /// Path to Cairo PIE file (.pie.zip)
        #[arg(long)]
        pie: PathBuf,

        /// Output path for the proof
        #[arg(long)]
        output: PathBuf,

        /// Bootloader program path
        #[arg(long)]
        bootloader: Option<PathBuf>,

        /// Prover parameters JSON
        #[arg(long)]
        params: Option<PathBuf>,

        /// Verify the proof after generation
        #[arg(long)]
        verify: bool,
    },
}

pub async fn run(args: ProveArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    match args.mode {
        ProveMode::VirtualOs {
            block_number,
            tx_hash,
            tx_json,
            rpc_url,
            output,
            prover_url,
            port,
            strk_fee_token,
        } => {
            run_virtual_os(
                block_number,
                tx_hash.as_deref(),
                tx_json.as_deref(),
                &rpc_url,
                &output,
                prover_url.as_deref(),
                port,
                strk_fee_token.as_deref(),
                env_file,
            )
            .await
        }
        ProveMode::Program {
            program,
            output,
            input,
            params,
            verify,
        } => run_program(&program, &output, input.as_deref(), params.as_deref(), verify, env_file).await,
        ProveMode::Pie {
            pie,
            output,
            bootloader,
            params,
            verify,
        } => {
            run_pie(
                &pie,
                &output,
                bootloader.as_deref(),
                params.as_deref(),
                verify,
                env_file,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_virtual_os(
    block_number: u64,
    tx_hash: Option<&str>,
    tx_json_path: Option<&std::path::Path>,
    rpc_url: &str,
    output: &std::path::Path,
    prover_url: Option<&str>,
    port: u16,
    strk_fee_token: Option<&str>,
    env_file: Option<&std::path::Path>,
) -> Result<()> {
    info!("=== Running Virtual OS ===");
    info!("  Block:  {block_number}");
    info!("  RPC:    {rpc_url}");
    info!("  Output: {}", output.display());

    let client = reqwest::Client::new();

    // Load transaction data: either from a JSON file or by fetching from RPC
    let tx_data: serde_json::Value = if let Some(path) = tx_json_path {
        info!("  Loading transaction from {}", path.display());
        let contents = tokio::fs::read_to_string(path)
            .await
            .wrap_err_with(|| format!("failed to read tx JSON from {}", path.display()))?;
        serde_json::from_str(&contents)
            .wrap_err("failed to parse transaction JSON")?
    } else if let Some(hash) = tx_hash {
        info!("  Fetching transaction {hash} from RPC...");
        let resp: serde_json::Value = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "starknet_getTransactionByHash",
                "params": {"transaction_hash": hash},
                "id": 1
            }))
            .send()
            .await
            .wrap_err("failed to fetch transaction")?
            .json()
            .await?;

        resp.get("result")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| eyre::eyre!("could not fetch transaction {hash}: {resp}"))?
    } else {
        bail!("either --tx-hash or --tx-json must be provided");
    };
    info!("  Transaction loaded successfully");

    let prove_endpoint;
    let mut _runner_child: Option<tokio::process::Child> = None;

    if let Some(url) = prover_url {
        prove_endpoint = url.to_string();
        info!("  Prover: {prove_endpoint} (remote)");
    } else {
        let config = Config::from_env(env_file)?;

        let runner_bin = config.runner_bin();
        if !runner_bin.exists() {
            bail!(
                "starknet_os_runner not found at {}. Run `snip36 setup` or provide --prover-url.",
                runner_bin.display()
            );
        }

        let mut cmd = tokio::process::Command::new(&runner_bin);

        // Run from the sequencer directory so resource files (prover_params.json etc.) are found
        let sequencer_dir = config.deps_dir.join("sequencer");
        cmd.current_dir(&sequencer_dir)
            .arg("--rpc-url")
            .arg(rpc_url)
            .arg("--chain-id")
            .arg("SN_SEPOLIA")
            .arg("--port")
            .arg(port.to_string())
            .arg("--ip")
            .arg("127.0.0.1")
            .arg("--skip-fee-field-validation");

        if let Some(strk_token) = strk_fee_token {
            cmd.arg("--strk-fee-token-address").arg(strk_token);
            info!("  STRK token: {strk_token}");
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        info!("Starting starknet_os_runner on port {port}...");
        let mut child = cmd.spawn().wrap_err("failed to start starknet_os_runner")?;

        // Stream stderr in background
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(target: "runner", "{line}");
                }
            });
        }

        // Wait for server to be ready
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner().template("{spinner} {msg}")?);
        pb.set_message("Waiting for runner to start...");

        let ready_url = format!("http://127.0.0.1:{port}/");
        for _ in 0..30 {
            if client.get(&ready_url).send().await.is_ok() {
                break;
            }
            // Check if process exited
            if let Some(status) = child.try_wait()? {
                bail!("starknet_os_runner exited prematurely with {status}");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            pb.tick();
        }
        pb.finish_with_message("Server ready");

        prove_endpoint = ready_url;
        _runner_child = Some(child);
    }

    // Call starknet_proveTransaction
    info!("Sending starknet_proveTransaction request...");
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::default_spinner().template("{spinner} {msg}")?);
    pb.set_message("Proving transaction (this may take several minutes)...");
    pb.enable_steady_tick(Duration::from_millis(200));

    let prove_response: serde_json::Value = client
        .post(&prove_endpoint)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "starknet_proveTransaction",
            "params": {
                "block_id": {"block_number": block_number},
                "transaction": tx_data
            },
            "id": 1
        }))
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .wrap_err("starknet_proveTransaction request failed")?
        .json()
        .await?;

    pb.finish_and_clear();

    // Check for errors
    if let Some(error) = prove_response.get("error") {
        bail!("starknet_proveTransaction failed: {error}");
    }

    let result = prove_response
        .get("result")
        .filter(|v| !v.is_null())
        .ok_or_else(|| eyre::eyre!("empty result from starknet_proveTransaction: {prove_response}"))?;

    // Save proof
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let proof = result
        .get("proof")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    tokio::fs::write(output, proof).await?;

    // Save proof_facts alongside
    let facts_output = output.with_extension("proof_facts");
    if let Some(facts) = result.get("proof_facts") {
        tokio::fs::write(&facts_output, facts.to_string()).await?;
    }

    // Save L2→L1 messages if present
    let messages_output = output.with_extension("raw_messages.json");
    if let Some(messages) = result.get("l2_to_l1_messages") {
        if let Some(arr) = messages.as_array() {
            if !arr.is_empty() {
                let messages_json = serde_json::json!({ "l2_to_l1_messages": messages });
                tokio::fs::write(
                    &messages_output,
                    serde_json::to_string_pretty(&messages_json)?,
                )
                .await?;
                info!("  L2→L1 messages: {} message(s) saved", arr.len());
            }
        }
    }

    info!("=== Virtual OS execution complete ===");
    info!("  Proof:       {}", output.display());
    info!("  Proof facts: {}", facts_output.display());
    if messages_output.exists() {
        info!("  Messages:    {}", messages_output.display());
    }

    if output.exists() {
        let metadata = tokio::fs::metadata(output).await?;
        info!("  Proof size:  {} bytes", metadata.len());
    }

    // Kill the runner if we started it
    if let Some(mut child) = _runner_child {
        let _ = child.kill().await;
    }

    Ok(())
}

async fn run_program(
    program: &std::path::Path,
    output: &std::path::Path,
    input: Option<&std::path::Path>,
    params: Option<&std::path::Path>,
    verify: bool,
    env_file: Option<&std::path::Path>,
) -> Result<()> {
    let config = Config::from_env(env_file)?;
    let prover_bin = config.prover_bin();

    if !prover_bin.exists() {
        bail!(
            "stwo-run-and-prove not found at {}. Run `snip36 setup` first.",
            prover_bin.display()
        );
    }

    if !program.exists() {
        bail!("program file not found: {}", program.display());
    }

    let program = program.canonicalize()?;
    let params_path = match params {
        Some(p) => p.canonicalize()?,
        None => config.prover_params().canonicalize()?,
    };

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let output_abs = std::path::absolute(output)?;

    info!("=== Proving Cairo Program ===");
    info!("  Program: {}", program.display());
    info!("  Params:  {}", params_path.display());
    info!("  Output:  {}", output_abs.display());

    let mut cmd = tokio::process::Command::new(&prover_bin);
    cmd.arg("--program")
        .arg(&program)
        .arg("--prover_params_json")
        .arg(&params_path)
        .arg("--proof_path")
        .arg(&output_abs);

    if let Some(input_path) = input {
        let input_abs = input_path.canonicalize()?;
        info!("  Input:   {}", input_abs.display());
        cmd.arg("--program_input").arg(&input_abs);
    }

    if verify {
        info!("  Verify:  yes");
        cmd.arg("--verify");
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::default_spinner().template("{spinner} {msg}")?);
    pb.set_message("Running prover...");
    pb.enable_steady_tick(Duration::from_millis(200));

    let status = spawn_and_stream(cmd).await?;
    pb.finish_and_clear();

    if !status.success() {
        bail!("prover exited with {status}");
    }

    info!("=== Proof generated ===");
    info!("  Location: {}", output_abs.display());

    if output_abs.exists() {
        let metadata = tokio::fs::metadata(&output_abs).await?;
        info!("  Size:     {} bytes", metadata.len());
    }

    Ok(())
}

async fn run_pie(
    pie: &std::path::Path,
    output: &std::path::Path,
    bootloader: Option<&std::path::Path>,
    params: Option<&std::path::Path>,
    verify: bool,
    env_file: Option<&std::path::Path>,
) -> Result<()> {
    let config = Config::from_env(env_file)?;
    let prover_bin = config.prover_bin();

    if !prover_bin.exists() {
        bail!(
            "stwo-run-and-prove not found at {}. Run `snip36 setup` first.",
            prover_bin.display()
        );
    }

    if !pie.exists() {
        bail!("PIE file not found: {}", pie.display());
    }

    let bootloader_path = match bootloader {
        Some(p) => p.to_path_buf(),
        None => config.bootloader_program(),
    };
    if !bootloader_path.exists() {
        bail!(
            "bootloader program not found at {}. Run `snip36 setup` first.",
            bootloader_path.display()
        );
    }

    let pie_abs = pie.canonicalize()?;
    let bootloader_abs = bootloader_path.canonicalize()?;
    let params_path = match params {
        Some(p) => p.canonicalize()?,
        None => config.prover_params().canonicalize()?,
    };

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let output_abs = std::path::absolute(output)?;

    // Generate bootloader input from template
    let bootloader_input = serde_json::json!({
        "simple_bootloader_input": {
            "tasks": [
                {
                    "RunProgramTask": {
                        "program_input_path": pie_abs.to_string_lossy()
                    }
                }
            ]
        }
    });

    let tmp_dir = tempfile::tempdir()?;
    let input_file = tmp_dir.path().join("bootloader_input.json");
    tokio::fs::write(&input_file, serde_json::to_string_pretty(&bootloader_input)?).await?;

    info!("=== Proving Cairo PIE via Bootloader ===");
    info!("  PIE:        {}", pie_abs.display());
    info!("  Bootloader: {}", bootloader_abs.display());
    info!("  Params:     {}", params_path.display());
    info!("  Output:     {}", output_abs.display());
    if verify {
        info!("  Verify:     yes");
    }

    let mut cmd = tokio::process::Command::new(&prover_bin);
    cmd.arg("--program")
        .arg(&bootloader_abs)
        .arg("--prover_params_json")
        .arg(&params_path)
        .arg("--program_input")
        .arg(&input_file)
        .arg("--proof_path")
        .arg(&output_abs);

    if verify {
        cmd.arg("--verify");
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::default_spinner().template("{spinner} {msg}")?);
    pb.set_message("Running prover via bootloader...");
    pb.enable_steady_tick(Duration::from_millis(200));

    let status = spawn_and_stream(cmd).await?;
    pb.finish_and_clear();

    if !status.success() {
        bail!("prover exited with {status}");
    }

    info!("=== Proof generated ===");
    info!("  Location: {}", output_abs.display());

    if output_abs.exists() {
        let metadata = tokio::fs::metadata(&output_abs).await?;
        info!("  Size:     {} bytes", metadata.len());
    }

    Ok(())
}

/// Spawn a command and stream its stdout/stderr to tracing.
async fn spawn_and_stream(
    mut cmd: tokio::process::Command,
) -> Result<std::process::ExitStatus> {
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().wrap_err("failed to spawn subprocess")?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_handle = tokio::spawn(async move {
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!(target: "prover", "{line}");
            }
        }
    });

    let stderr_handle = tokio::spawn(async move {
        if let Some(stderr) = stderr {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                error!(target: "prover", "{line}");
            }
        }
    });

    let status = child.wait().await?;
    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    Ok(status)
}
