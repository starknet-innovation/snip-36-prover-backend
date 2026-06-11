use std::path::Path;
use std::time::Duration;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, error, info};

use snip36_core::Config;

// When bumping this, regenerate vendor/proving-utils.Cargo.lock from the new commit.
const PROVING_UTILS_VERSION: &str = "c0b937bb19126255fbeeededbcaea4a84ae9f1c0";
const SEQUENCER_TAG: &str = "PRIVACY-0.14.2-RC.6";
const STWO_NIGHTLY: &str = "nightly-2025-07-14";
const RUNNER_PACKAGE: &str = "starknet_transaction_prover";
const RUNNER_BINARY: &str = "starknet_transaction_prover";

/// GitHub repo hosting the prebuilt deps releases (override: SNIP36_DEPS_REPO).
const DEPS_REPO: &str = "starknet-innovation/snip-36-prover-backend";
/// The deps-v* release this snip36 build expects. Single source of truth:
/// the `deps-version` file at the repo root, baked in by build.rs (and read
/// by scripts/download-deps.sh and daily-health.yml). Bump that file when
/// cutting a new deps-v* (see RELEASING.md).
pub const EXPECTED_DEPS_TAG: &str = env!("SNIP36_EXPECTED_DEPS_TAG");

const PROVING_UTILS_LOCKFILE: &[u8] = include_bytes!("../../../../vendor/proving-utils.Cargo.lock");

#[derive(Args)]
pub struct SetupArgs {
    /// Download prebuilt deps from the pinned GitHub release instead of
    /// building from source (~30s instead of ~30min; supported on
    /// darwin-arm64, linux-x86_64, linux-arm64)
    #[arg(long, conflicts_with_all = ["skip_prover", "skip_runner"])]
    prebuilt: bool,

    /// Skip building stwo-run-and-prove
    #[arg(long)]
    skip_prover: bool,

    /// Skip building starknet_transaction_prover
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
            strk_token: snip36_core::types::STRK_TOKEN.to_string(),
            output_dir: project_dir.join("output"),
            deps_dir: project_dir.join("deps"),
            project_dir,
        }
    });

    if args.prebuilt {
        return run_prebuilt(&config).await;
    }

    let deps_dir = &config.deps_dir;
    let project_dir = &config.project_dir;

    info!("=== SNIP-36 Virtual OS Stwo Prover -- Setup ===");
    info!("");

    // [1/7] Check Rust toolchains
    info!("[1/7] Checking Rust toolchains...");
    check_rust_toolchain().await?;
    info!("  Installing {STWO_NIGHTLY} (required by stwo 2.2.0)...");
    run_cmd("rustup", &["toolchain", "install", STWO_NIGHTLY]).await?;
    info!("");

    // [2/7] Clone/update proving-utils
    info!("[2/7] Setting up proving-utils...");
    tokio::fs::create_dir_all(deps_dir).await?;
    let proving_utils_dir = deps_dir.join("proving-utils");

    ensure_repo(
        &proving_utils_dir,
        "https://github.com/starkware-libs/proving-utils.git",
    )
    .await?;

    run_cmd_in(
        "git",
        &["fetch", "--quiet", "--tags", "origin"],
        &proving_utils_dir,
    )
    .await?;

    info!("  Checking out {PROVING_UTILS_VERSION}...");
    run_cmd_in(
        "git",
        &["checkout", PROVING_UTILS_VERSION, "--quiet"],
        &proving_utils_dir,
    )
    .await?;

    // Upstream gitignores Cargo.lock; write our vendored one for reproducible builds.
    tokio::fs::write(proving_utils_dir.join("Cargo.lock"), PROVING_UTILS_LOCKFILE).await?;

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

    let sequencer_was_cloned = sequencer_dir.join(".git").exists();
    ensure_repo(
        &sequencer_dir,
        "https://github.com/starkware-libs/sequencer.git",
    )
    .await?;
    if !sequencer_was_cloned {
        run_cmd_in(
            "git",
            &["fetch", "--quiet", "--tags", "origin"],
            &sequencer_dir,
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
    let requirements = sequencer_dir.join("scripts/requirements.txt");
    let venv_dir = ensure_venv(project_dir, &requirements).await?;
    info!("");

    // [5/7] Build stwo-run-and-prove
    if !args.skip_prover {
        info!("[5/7] Building stwo-run-and-prove...");
        let bin_dir = deps_dir.join("bin");
        tokio::fs::create_dir_all(&bin_dir).await?;

        info!(
            "  Building in {} (this may take several minutes)...",
            proving_utils_dir.display()
        );
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
                "--locked",
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
            for line in stderr
                .lines()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
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
            info!(
                "  WARNING: stwo-run-and-prove --help returned non-zero (may still be functional)."
            );
        }
    }

    let runner_bin = sequencer_dir.join(format!("target/release/{RUNNER_BINARY}"));
    if runner_bin.exists() {
        let check = tokio::process::Command::new(&runner_bin)
            .arg("--help")
            .output()
            .await;
        if check.map(|o| o.status.success()).unwrap_or(false) {
            info!("  {RUNNER_BINARY}: OK");
        } else {
            info!("  WARNING: {RUNNER_BINARY} not functional.");
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

/// Provision the proving stack from the pinned prebuilt GitHub release
/// instead of building from source (mirrors scripts/download-deps.sh).
async fn run_prebuilt(config: &Config) -> Result<()> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        (os, arch) => bail!(
            "no prebuilt deps for {os}/{arch} (supported: darwin-arm64, linux-x86_64, \
             linux-arm64); run `snip36 setup` to build from source"
        ),
    };

    let repo = std::env::var("SNIP36_DEPS_REPO").unwrap_or_else(|_| DEPS_REPO.to_string());
    let asset = format!("snip36-deps-{platform}.tar.gz");
    let base_url = format!("https://github.com/{repo}/releases/download/{EXPECTED_DEPS_TAG}");

    info!("=== SNIP-36 Virtual OS Stwo Prover -- Setup (prebuilt) ===");
    info!("  Platform: {platform}");
    info!("  Release:  {EXPECTED_DEPS_TAG}");
    info!("");

    let deps_dir = &config.deps_dir;
    let bin_dir = deps_dir.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;

    // [1/4] Download (and verify when the release publishes SHA256SUMS)
    info!("[1/4] Downloading {asset}...");
    let tmp = tempfile::tempdir()?;
    let tar_path = tmp.path().join(&asset);
    download_to_file(&format!("{base_url}/{asset}"), &tar_path).await?;

    match fetch_text(&format!("{base_url}/SHA256SUMS")).await {
        Ok(sums) => verify_sha256(&tar_path, &asset, &sums).await?,
        Err(_) => info!(
            "  WARNING: no SHA256SUMS published for {EXPECTED_DEPS_TAG}; \
             skipping checksum verification"
        ),
    }

    // [2/4] Extract and lay out binaries where Config expects them
    info!("[2/4] Extracting...");
    run_cmd(
        "tar",
        &[
            "xzf",
            &tar_path.to_string_lossy(),
            "-C",
            &bin_dir.to_string_lossy(),
        ],
    )
    .await?;
    layout_prebuilt(deps_dir).await?;
    tokio::fs::write(
        deps_dir.join(".deps-version"),
        format!("{EXPECTED_DEPS_TAG}\n"),
    )
    .await?;
    info!("");

    // [3/4] Python venv for cairo-compile. The deps tarball ships binaries
    // only, so fetch the matching requirements.txt from the pinned sequencer
    // tag when no checkout is present.
    info!("[3/4] Setting up Python virtual environment...");
    let requirements = deps_dir.join("sequencer/scripts/requirements.txt");
    if !requirements.exists() {
        info!("  Fetching sequencer requirements.txt ({SEQUENCER_TAG})...");
        let url = format!(
            "https://raw.githubusercontent.com/starkware-libs/sequencer/{SEQUENCER_TAG}/scripts/requirements.txt"
        );
        let text = fetch_text(&url)
            .await
            .wrap_err("failed to fetch sequencer requirements.txt")?;
        if let Some(parent) = requirements.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&requirements, text).await?;
    }
    let venv_dir = ensure_venv(&config.project_dir, &requirements).await?;
    info!("");

    // [4/4] Verify
    info!("[4/4] Verifying...");
    let prover_bin = config.prover_bin();
    let runner_bin = config.runner_bin();
    for (name, path) in [
        ("stwo-run-and-prove", &prover_bin),
        (RUNNER_BINARY, &runner_bin),
    ] {
        if !path.exists() {
            bail!("{name} missing at {} after extraction", path.display());
        }
        let ok = tokio::process::Command::new(path)
            .arg("--help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            info!("  {name}: OK");
        } else {
            info!("  WARNING: {name} --help returned non-zero (may still be functional).");
        }
    }
    let bootloader = config.bootloader_program();
    if !bootloader.exists() {
        bail!(
            "bootloader_program.json missing at {} after extraction",
            bootloader.display()
        );
    }
    info!("  bootloader_program.json: OK");

    info!("");
    info!("=== Setup complete (prebuilt {EXPECTED_DEPS_TAG}) ===");
    info!("");
    info!("  Prover binary: {}", prover_bin.display());
    info!("  Runner binary: {}", runner_bin.display());
    info!("  Python venv:   {}", venv_dir.display());
    info!("");
    info!("  Next steps:");
    info!("    1. cp .env.example .env  # Configure account credentials");
    info!("    2. snip36 doctor         # Re-check the stack any time");
    info!("    3. snip36 e2e            # Run full E2E test");

    Ok(())
}

/// Clone `url` into `dir`, tolerating a pre-existing non-empty directory
/// without `.git` — CI restores `<dir>/target` from cache before setup runs,
/// and `git clone` refuses to clone into a non-empty directory. Initialize
/// the repo in place instead and let the caller fetch + checkout.
async fn ensure_repo(dir: &Path, url: &str) -> Result<()> {
    if dir.join(".git").exists() {
        info!("  Already cloned at {}", dir.display());
    } else if dir.exists() {
        info!(
            "  Initializing git in non-empty {} (restored from cache)...",
            dir.display()
        );
        run_cmd_in("git", &["init", "--quiet"], dir).await?;
        run_cmd_in("git", &["remote", "add", "origin", url], dir).await?;
    } else {
        info!("  Cloning {url}...");
        run_cmd("git", &["clone", url, &dir.to_string_lossy()]).await?;
    }
    Ok(())
}

/// Prefer python3.12 (cairo-lang requires <3.13), fall back to python3.
async fn pick_python() -> &'static str {
    if tokio::process::Command::new("python3.12")
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
    }
}

/// A venv is not relocatable: pyvenv.cfg and console-script shebangs embed
/// absolute paths, so a moved project directory or an upgraded/removed base
/// interpreter leaves a venv that fails with "bad interpreter" at prove time.
/// Probe a console script (whose shebang exercises both) instead of trusting
/// file existence.
pub async fn venv_is_healthy(venv_dir: &Path) -> bool {
    tokio::process::Command::new(venv_dir.join("bin/pip"))
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Create (or recreate, when stale) sequencer_venv and install the sequencer
/// Python requirements into it. Returns the venv path.
async fn ensure_venv(project_dir: &Path, requirements: &Path) -> Result<std::path::PathBuf> {
    let venv_dir = project_dir.join("sequencer_venv");
    let python_bin = pick_python().await;

    if venv_dir.join("bin/pip").exists() && venv_is_healthy(&venv_dir).await {
        info!("  Venv already exists at {}", venv_dir.display());
    } else {
        if venv_dir.exists() {
            info!(
                "  sequencer_venv is stale or incomplete (interpreter missing or \
                 project moved); recreating..."
            );
            tokio::fs::remove_dir_all(&venv_dir).await?;
        }
        info!("  Creating venv with {python_bin}...");
        run_cmd(python_bin, &["-m", "venv", &venv_dir.to_string_lossy()]).await?;
    }

    info!("  Installing Python dependencies...");
    let pip = venv_dir.join("bin/pip");
    if requirements.exists() {
        run_cmd(
            pip.to_str().unwrap_or("pip"),
            &["install", "--quiet", "-r", &requirements.to_string_lossy()],
        )
        .await
        .wrap_err(
            "failed to install sequencer Python requirements (cairo-lang needs Python <3.13)",
        )?;
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
    Ok(venv_dir)
}

/// Stream a URL to a local file with a progress bar.
async fn download_to_file(url: &str, dest: &Path) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    debug!("Downloading {url}");
    let mut resp = reqwest::get(url)
        .await
        .wrap_err_with(|| format!("failed to fetch {url}"))?
        .error_for_status()
        .wrap_err_with(|| format!("failed to fetch {url}"))?;

    let pb = match resp.content_length() {
        Some(len) => {
            let pb = ProgressBar::new(len);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  {bar:30} {bytes}/{total_bytes} ({bytes_per_sec})")?,
            );
            pb
        }
        None => ProgressBar::new_spinner(),
    };

    let mut file = tokio::fs::File::create(dest).await?;
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk).await?;
        pb.inc(chunk.len() as u64);
    }
    file.flush().await?;
    pb.finish_and_clear();
    Ok(())
}

async fn fetch_text(url: &str) -> Result<String> {
    Ok(reqwest::get(url).await?.error_for_status()?.text().await?)
}

/// Verify `file` against its entry in a SHA256SUMS body. A missing entry
/// warns rather than fails (older releases predate checksums); a mismatch
/// is fatal.
async fn verify_sha256(file: &Path, asset: &str, sums: &str) -> Result<()> {
    use sha2::Digest;

    let expected = sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        (name == asset).then(|| hash.to_lowercase())
    });
    let Some(expected) = expected else {
        info!("  WARNING: SHA256SUMS has no entry for {asset}; skipping checksum verification");
        return Ok(());
    };

    let bytes = tokio::fs::read(file).await?;
    let actual = hex::encode(sha2::Sha256::digest(&bytes));
    if actual != expected {
        bail!("checksum mismatch for {asset}: expected {expected}, got {actual}");
    }
    info!("  Checksum verified.");
    Ok(())
}

/// Move extracted binaries from deps/bin into the layout Config expects
/// (mirrors scripts/download-deps.sh).
async fn layout_prebuilt(deps_dir: &Path) -> Result<()> {
    let bin_dir = deps_dir.join("bin");
    let release_dir = deps_dir.join("sequencer/target/release");
    tokio::fs::create_dir_all(&release_dir).await?;

    // The current CLI expects starknet_transaction_prover; keep the
    // starknet_os_runner alias for older scripts.
    for name in ["starknet_transaction_prover", "starknet_os_runner"] {
        let src = bin_dir.join(name);
        if src.exists() {
            tokio::fs::rename(&src, release_dir.join(name)).await?;
        }
    }
    let tx_prover = release_dir.join("starknet_transaction_prover");
    let os_runner = release_dir.join("starknet_os_runner");
    if tx_prover.exists() && !os_runner.exists() {
        tokio::fs::copy(&tx_prover, &os_runner).await?;
    }
    if os_runner.exists() && !tx_prover.exists() {
        tokio::fs::copy(&os_runner, &tx_prover).await?;
    }

    // starknet-sierra-compile: deps-v4+ tarballs ship it flat at
    // shared_executables/; older tags nest it under shared_executables/bin/.
    // Accept both.
    let sierra_dir = release_dir.join("shared_executables");
    tokio::fs::create_dir_all(&sierra_dir).await?;
    for src in [
        bin_dir.join("shared_executables/starknet-sierra-compile"),
        bin_dir.join("shared_executables/bin/starknet-sierra-compile"),
    ] {
        if src.exists() {
            tokio::fs::rename(&src, sierra_dir.join("starknet-sierra-compile")).await?;
            break;
        }
    }
    let _ = tokio::fs::remove_dir_all(bin_dir.join("shared_executables")).await;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [
            bin_dir.join("stwo-run-and-prove"),
            tx_prover,
            os_runner,
            sierra_dir.join("starknet-sierra-compile"),
        ] {
            if path.exists() {
                tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await?;
            }
        }
    }
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
                .args([
                    "-c",
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                ])
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

    let mut child = cmd
        .spawn()
        .wrap_err_with(|| format!("failed to run {program}"))?;

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

    let rlimit_file = sequencer_dir
        .join("crates/apollo_compilation_utils/src/resource_limits/resource_limits_unix.rs");

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
