use std::path::Path;
use std::time::Duration;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, error, info};

use snip36_core::Config;

const PROVING_UTILS_VERSION: &str = "main";
const SEQUENCER_TAG: &str = "PRIVACY-0.14.2-RC.2";
const STWO_NIGHTLY: &str = "nightly-2025-07-14";
const RUNNER_PACKAGE: &str = "starknet_transaction_prover";
const RUNNER_BINARY: &str = "starknet_transaction_prover";

#[derive(Args)]
pub struct SetupArgs {
    /// Skip building stwo-run-and-prove
    #[arg(long)]
    skip_prover: bool,

    /// Skip building starknet_os_runner
    #[arg(long)]
    skip_runner: bool,
}

pub async fn run(args: SetupArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    let config = Config::from_env(env_file).unwrap_or_else(|_| {
        // Fall back to reasonable defaults if env vars are missing
        let project_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());
        Config {
            rpc_url: String::new(),
            account_address: String::new(),
            private_key: String::new(),
            chain_id: "SN_SEPOLIA".into(),
            gateway_url: None,
            output_dir: project_dir.join("output"),
            deps_dir: project_dir.join("deps"),
            project_dir,
        }
    });

    let deps_dir = &config.deps_dir;
    let project_dir = &config.project_dir;

    info!("=== SNIP-36 Virtual OS Stwo Prover -- Setup ===");
    info!("");

    // [1/7] Check Rust toolchains
    info!("[1/7] Checking Rust toolchains...");
    check_rust_toolchain().await?;
    info!("  Installing {STWO_NIGHTLY} (required by stwo 2.1.0)...");
    run_cmd("rustup", &["toolchain", "install", STWO_NIGHTLY]).await?;
    info!("");

    // [2/7] Clone/update proving-utils
    info!("[2/7] Setting up proving-utils...");
    tokio::fs::create_dir_all(deps_dir).await?;
    let proving_utils_dir = deps_dir.join("proving-utils");

    if proving_utils_dir.join(".git").exists() {
        info!("  Already cloned at {}", proving_utils_dir.display());
    } else {
        info!("  Cloning starkware-libs/proving-utils...");
        run_cmd(
            "git",
            &[
                "clone",
                "https://github.com/starkware-libs/proving-utils.git",
                &proving_utils_dir.to_string_lossy(),
            ],
        )
        .await?;
    }

    info!("  Checking out {PROVING_UTILS_VERSION}...");
    run_cmd_in(
        "git",
        &["checkout", PROVING_UTILS_VERSION, "--quiet"],
        &proving_utils_dir,
    )
    .await?;

    // Install the Rust nightly from proving-utils
    let toolchain_file = proving_utils_dir.join("rust-toolchain.toml");
    if toolchain_file.exists() {
        let content = tokio::fs::read_to_string(&toolchain_file).await?;
        if let Some(nightly) = extract_channel(&content) {
            info!("  Installing Rust toolchain: {nightly}");
            run_cmd("rustup", &["toolchain", "install", &nightly]).await?;
            run_cmd(
                "rustup",
                &[
                    "override",
                    "set",
                    &nightly,
                    "--path",
                    &proving_utils_dir.to_string_lossy(),
                ],
            )
            .await?;
        }
    }
    info!("");

    // [3/7] Clone/update sequencer
    info!("[3/7] Setting up sequencer...");
    let sequencer_dir = deps_dir.join("sequencer");

    if sequencer_dir.join(".git").exists() {
        info!("  Already cloned at {}", sequencer_dir.display());
    } else {
        info!("  Cloning starkware-libs/sequencer...");
        run_cmd(
            "git",
            &[
                "clone",
                "https://github.com/starkware-libs/sequencer.git",
                &sequencer_dir.to_string_lossy(),
            ],
        )
        .await?;
    }

    info!("  Checking out {SEQUENCER_TAG}...");
    run_cmd_in(
        "git",
        &["checkout", SEQUENCER_TAG, "--quiet"],
        &sequencer_dir,
    )
    .await?;

    // Apply macOS RLIMIT_AS patch
    apply_macos_patch(&sequencer_dir).await?;
    info!("");

    // [4/7] Python venv
    info!("[4/7] Setting up Python virtual environment...");
    let venv_dir = project_dir.join("sequencer_venv");

    // Prefer python3.12 (cairo-lang requires <3.13), fall back to python3
    let python_bin = if tokio::process::Command::new("python3.12")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
    {
        "python3.12"
    } else {
        "python3"
    };

    if venv_dir.join("bin/pip").exists() {
        info!("  Venv already exists at {}", venv_dir.display());
    } else {
        info!("  Creating venv with {python_bin}...");
        run_cmd(python_bin, &["-m", "venv", &venv_dir.to_string_lossy()]).await?;
    }

    info!("  Installing Python dependencies...");
    let pip = venv_dir.join("bin/pip");
    let requirements = sequencer_dir.join("scripts/requirements.txt");
    if requirements.exists() {
        run_cmd(
            pip.to_str().unwrap_or("pip"),
            &["install", "--quiet", "-r", &requirements.to_string_lossy()],
        )
        .await
        .wrap_err("failed to install sequencer Python requirements (cairo-lang needs Python <3.13)")?;
    }

    // Verify cairo-compile is available
    let cairo_compile = venv_dir.join("bin/cairo-compile");
    if !cairo_compile.exists() {
        bail!(
            "cairo-compile not found in venv after pip install. \
             The sequencer build requires cairo-lang which needs Python <3.13. \
             You have: {python_bin}"
        );
    }
    info!("  cairo-compile: {}", cairo_compile.display());
    info!("");

    // [5/7] Build stwo-run-and-prove
    if !args.skip_prover {
        info!("[5/7] Building stwo-run-and-prove...");
        let bin_dir = deps_dir.join("bin");
        tokio::fs::create_dir_all(&bin_dir).await?;

        info!("  Building in {} (this may take several minutes)...", proving_utils_dir.display());
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner().template("{spinner} {msg}")?);
        pb.set_message("Building stwo-run-and-prove...");
        pb.enable_steady_tick(Duration::from_millis(200));

        run_cmd(
            "cargo",
            &[
                &format!("+{STWO_NIGHTLY}"),
                "build",
                "--release",
                "--manifest-path",
                &proving_utils_dir.join("Cargo.toml").to_string_lossy(),
                "-p",
                "stwo-run-and-prove",
            ],
        )
        .await?;
        pb.finish_and_clear();

        // Copy binary
        let src = proving_utils_dir.join("target/release/stwo-run-and-prove");
        let dst = bin_dir.join("stwo-run-and-prove");
        if src.exists() {
            tokio::fs::copy(&src, &dst).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755)).await?;
            }
            info!("  Binary: {}", dst.display());
        } else {
            bail!("stwo-run-and-prove binary not found after build");
        }
    } else {
        info!("[5/7] Skipping stwo-run-and-prove build (--skip-prover)");
    }
    info!("");

    // [6/7] Build starknet_transaction_prover
    if !args.skip_runner {
        info!("[6/7] Building {RUNNER_PACKAGE} (requires {STWO_NIGHTLY} + venv)...");
        info!("  This requires the stwo_proving feature and may take several minutes...");

        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner().template("{spinner} {msg}")?);
        pb.set_message(format!("Building {RUNNER_PACKAGE}..."));
        pb.enable_steady_tick(Duration::from_millis(200));

        let venv_bin = venv_dir.join("bin");
        let path_env = format!(
            "{}:{}",
            venv_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let output = tokio::process::Command::new("cargo")
            .args([
                &format!("+{STWO_NIGHTLY}"),
                "build",
                "--release",
                "--manifest-path",
                &sequencer_dir.join("Cargo.toml").to_string_lossy(),
                "-p",
                RUNNER_PACKAGE,
                "--features",
                "stwo_proving",
            ])
            .env("PATH", &path_env)
            .output()
            .await
            .wrap_err(format!("failed to build {RUNNER_PACKAGE}"))?;

        pb.finish_and_clear();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Show last 30 lines of build errors
            for line in stderr.lines().rev().take(30).collect::<Vec<_>>().into_iter().rev() {
                error!("  {line}");
            }
            bail!("{RUNNER_PACKAGE} build failed");
        }

        let runner_bin = sequencer_dir.join(format!("target/release/{RUNNER_BINARY}"));
        if runner_bin.exists() {
            info!("  Binary: {}", runner_bin.display());
        } else {
            bail!("{RUNNER_BINARY} binary not found after build");
        }
    } else {
        info!("[6/7] Skipping {RUNNER_PACKAGE} build (--skip-runner)");
    }
    info!("");

    // [7/7] Copy bootloader program
    info!("[7/7] Locating bootloader program...");
    let bin_dir = deps_dir.join("bin");
    let bootloader_dst = bin_dir.join("bootloader_program.json");

    // Search for the bootloader in proving-utils resources
    let proving_utils_dir = deps_dir.join("proving-utils");
    let bootloader_found = find_bootloader(&proving_utils_dir).await;

    match bootloader_found {
        Some(src) => {
            tokio::fs::copy(&src, &bootloader_dst).await?;
            info!("  Bootloader program: {}", bootloader_dst.display());
        }
        None => {
            error!("  WARNING: Bootloader program not found in proving-utils resources.");
        }
    }
    info!("");

    // Verify
    info!("=== Verification ===");
    let prover_bin = bin_dir.join("stwo-run-and-prove");
    if prover_bin.exists() {
        let check = tokio::process::Command::new(&prover_bin)
            .arg("--help")
            .output()
            .await;
        if check.map(|o| o.status.success()).unwrap_or(false) {
            info!("  stwo-run-and-prove: OK");
        } else {
            info!("  WARNING: stwo-run-and-prove --help returned non-zero (may still be functional).");
        }
    }

    let runner_bin = sequencer_dir.join("target/release/starknet_os_runner");
    if runner_bin.exists() {
        let check = tokio::process::Command::new(&runner_bin)
            .arg("--help")
            .output()
            .await;
        if check.map(|o| o.status.success()).unwrap_or(false) {
            info!("  starknet_os_runner: OK");
        } else {
            info!("  WARNING: starknet_os_runner not functional.");
        }
    }

    info!("");
    info!("=== Setup complete ===");
    info!("");
    info!("  Prover binary: {}", prover_bin.display());
    info!("  Runner binary: {}", runner_bin.display());
    info!("  Python venv:   {}", venv_dir.display());
    info!("");
    info!("  Next steps:");
    info!("    1. cp .env.example .env  # Configure account credentials");
    info!("    2. source .env && export STARKNET_RPC_URL STARKNET_ACCOUNT_ADDRESS STARKNET_PRIVATE_KEY");
    info!("    3. snip36 e2e             # Run full E2E test");

    Ok(())
}

async fn check_rust_toolchain() -> Result<()> {
    let output = tokio::process::Command::new("rustup")
        .arg("--version")
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let version = String::from_utf8_lossy(&o.stdout);
            info!("  rustup: {}", version.trim());
            Ok(())
        }
        _ => {
            info!("  Installing rustup...");
            let status = tokio::process::Command::new("sh")
                .args(["-c", "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"])
                .status()
                .await
                .wrap_err("failed to install rustup")?;
            if !status.success() {
                bail!("rustup installation failed");
            }
            Ok(())
        }
    }
}

async fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    debug!("Running: {program} {}", args.join(" "));
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().wrap_err_with(|| format!("failed to run {program}"))?;

    // Collect stderr so we can show it on failure
    let stderr_handle = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut collected = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "setup", "{line}");
                collected.push(line);
            }
            collected
        })
    });

    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "setup", "{line}");
            }
        });
    }

    let status = child.wait().await?;
    if !status.success() {
        // Show the last 30 lines of stderr so errors are visible without --verbose
        if let Some(handle) = stderr_handle {
            if let Ok(lines) = handle.await {
                let tail: Vec<_> = lines.iter().rev().take(30).collect();
                for line in tail.into_iter().rev() {
                    error!("  {line}");
                }
            }
        }
        bail!("{program} exited with {status}");
    }
    Ok(())
}

async fn run_cmd_in(program: &str, args: &[&str], dir: &Path) -> Result<()> {
    debug!("Running in {}: {program} {}", dir.display(), args.join(" "));
    let status = tokio::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status()
        .await
        .wrap_err_with(|| format!("failed to run {program}"))?;
    if !status.success() {
        bail!("{program} exited with {status}");
    }
    Ok(())
}

fn extract_channel(toml_content: &str) -> Option<String> {
    for line in toml_content.lines() {
        if line.contains("channel") {
            let parts: Vec<&str> = line.split('"').collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
    }
    None
}

async fn apply_macos_patch(sequencer_dir: &Path) -> Result<()> {
    if std::env::consts::OS != "macos" {
        return Ok(());
    }

    let rlimit_file = sequencer_dir.join(
        "crates/apollo_compilation_utils/src/resource_limits/resource_limits_unix.rs",
    );

    if !rlimit_file.exists() {
        return Ok(());
    }

    let content = tokio::fs::read_to_string(&rlimit_file).await?;
    if content.contains("target_os") {
        info!("  macOS RLIMIT_AS patch: already applied");
        return Ok(());
    }

    info!("  Applying macOS RLIMIT_AS fix...");

    let old = r#"memory_size: memory_size.map(|y| RLimit {
                resource: Resource::AS,
                soft_limit: y,
                hard_limit: y,
                units: "bytes".to_string(),
            }),"#;

    let new = r#"// macOS does not support RLIMIT_AS; skip on Apple targets.
            memory_size: if cfg!(target_os = "macos") {
                None
            } else {
                memory_size.map(|y| RLimit {
                    resource: Resource::AS,
                    soft_limit: y,
                    hard_limit: y,
                    units: "bytes".to_string(),
                })
            },"#;

    let patched = content.replace(old, new);
    tokio::fs::write(&rlimit_file, patched).await?;
    info!("  Patched.");

    Ok(())
}

async fn find_bootloader(proving_utils_dir: &Path) -> Option<std::path::PathBuf> {
    // Walk for simple_bootloader*.json in resources/
    let resources_pattern = proving_utils_dir.join("**");
    let walker = walkdir_async(resources_pattern.parent()?).await;
    walker
}

/// Simple recursive search for a file matching the bootloader pattern.
async fn walkdir_async(base: &Path) -> Option<std::path::PathBuf> {
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("simple_bootloader")
                    && name.ends_with(".json")
                    && path.to_string_lossy().contains("resources")
                {
                    return Some(path);
                }
            }
        }
    }
    None
}
