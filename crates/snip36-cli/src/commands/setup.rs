use std::path::Path;
use std::time::Duration;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, error, info};

use snip36_core::Config;

// When bumping this, regenerate vendor/proving-utils.Cargo.lock from the new commit.
const PROVING_UTILS_VERSION: &str = "0a97f45a15809e7b5a821b7adc49212af7b399f1";
const SEQUENCER_TAG: &str = "148de44521b49e32c5a675e287572768932cc99e";
const STWO_NIGHTLY: &str = "nightly-2026-01-15";
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

/// The low-memory prover: the provably.fast memory frontier (starknet-innovation/proving
/// 77964886) backported onto the stwo stack SEQUENCER_TAG pins. It changes how the prover
/// schedules and frees memory. V1 and V2 each have a patch set for their exact pins. See vendor/low-memory-prover/README.md.
///
/// (repo, clone URL, the commit the sequencer's Cargo.lock pins, patch, [patch] entries: package
/// name and its path inside the repo).
type LowMemoryRepo = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);
const LOW_MEMORY_PROVER: &[LowMemoryRepo] = &[
    (
        "stwo",
        "https://github.com/starkware-libs/stwo",
        "489a0f3ee44a59e03944ad9aa4f3e5a91a3d3f08",
        include_str!("../../../../vendor/low-memory-prover/stwo.patch"),
        &[
            ("stwo", "crates/stwo"),
            ("stwo-constraint-framework", "crates/constraint-framework"),
            ("stwo-air-utils", "crates/air-utils"),
            ("stwo-air-utils-derive", "crates/air-utils-derive"),
        ],
    ),
    (
        "stwo-cairo",
        "https://github.com/starkware-libs/stwo-cairo",
        "9b6be2710e4d7124bc93fe80a5af36556d42ed29",
        include_str!("../../../../vendor/low-memory-prover/stwo-cairo.patch"),
        &[
            ("cairo-air", "stwo_cairo_prover/crates/cairo-air"),
            ("stwo-cairo-adapter", "stwo_cairo_prover/crates/adapter"),
            ("stwo-cairo-prover", "stwo_cairo_prover/crates/prover"),
            (
                "stwo-cairo-serialize",
                "stwo_cairo_prover/crates/cairo-serialize",
            ),
            (
                "stwo-cairo-serialize-derive",
                "stwo_cairo_prover/crates/cairo-serialize-derive",
            ),
            ("stwo-cairo-common", "stwo_cairo_prover/crates/common"),
        ],
    ),
    (
        "stwo-circuits",
        "https://github.com/starkware-libs/stwo-circuits",
        "5ef951a12727e3489a2ee991c580a5b925c59c69",
        include_str!("../../../../vendor/low-memory-prover/stwo-circuits.patch"),
        &[
            ("circuits", "crates/circuits"),
            ("circuit-common", "crates/circuit_common"),
            ("circuit-prover", "crates/circuit_prover"),
            ("circuit-serialize", "crates/circuit_serialize"),
            ("circuit-verifier", "crates/circuit_verifier"),
            ("circuit-cairo-verifier", "crates/cairo_verifier"),
            ("circuits-stark-verifier", "crates/stark_verifier"),
        ],
    ),
    (
        "proving-utils",
        "https://github.com/starkware-libs/proving-utils",
        "3035dd00421daa541894297bd754db6e2787807b",
        include_str!("../../../../vendor/low-memory-prover/proving-utils.patch"),
        &[
            ("privacy-prove", "crates/privacy_prove"),
            ("privacy-circuit-verify", "crates/privacy_circuit_verify"),
            (
                "cairo-program-runner-lib",
                "crates/cairo-program-runner-lib",
            ),
        ],
    ),
];

const LOW_MEMORY_PROVER_V2: &[LowMemoryRepo] = &[(
    "proving-v2",
    "https://github.com/starkware-libs/proving",
    "e0035ffc8e08da270b8cb4a71f3708af70c2c775",
    include_str!("../../../../vendor/low-memory-prover/proving-v2.patch"),
    &[
        ("stwo", "crates/stwo"),
        ("stwo-air-utils", "crates/air-utils"),
        ("stwo-air-utils-derive", "crates/air-utils-derive"),
        ("stwo-constraint-framework", "crates/constraint-framework"),
        ("stwo-cairo-adapter", "crates/adapter"),
        ("stwo-cairo-common", "crates/common"),
        ("stwo-cairo-serialize", "crates/cairo-serialize"),
        (
            "stwo-cairo-serialize-derive",
            "crates/cairo-serialize-derive",
        ),
        ("cairo-air", "crates/cairo-air"),
        ("stwo-cairo-prover", "crates/prover"),
        ("stwo-cairo-utils", "crates/utils"),
        ("circuit-cairo-verifier", "crates/cairo_verifier"),
        ("circuit-common", "crates/circuit_common"),
        ("circuits", "crates/circuits"),
        ("circuits-stark-verifier", "crates/stark_verifier"),
        ("circuit-prover", "crates/circuit_prover"),
        ("circuit-verifier", "crates/circuit_verifier"),
        ("circuit-serialize", "crates/circuit_serialize"),
        (
            "cairo-program-runner-lib",
            "crates/cairo-program-runner-lib",
        ),
        ("leaf-proof-format", "crates/leaf_proof_format"),
        ("circuit-registry", "crates/circuit_registry"),
        ("leaf-prover", "crates/leaf_prover"),
        ("privacy-circuit-verify", "crates/privacy_circuit_verify"),
        ("privacy_common", "crates/privacy_common"),
        ("privacy-prove", "crates/privacy_prove"),
    ],
)];

#[derive(Args)]
pub struct SetupArgs {
    /// Download prebuilt deps from the pinned GitHub release instead of
    /// building from source (~30s instead of ~30min; supported on
    /// darwin-arm64, linux-x86_64, linux-arm64)
    #[arg(long, conflicts_with_all = ["skip_prover", "skip_runner", "stock_prover", "low_memory_prover"])]
    prebuilt: bool,

    /// Skip building stwo-run-and-prove
    #[arg(long)]
    skip_prover: bool,

    /// Skip building starknet_transaction_prover
    #[arg(long)]
    skip_runner: bool,

    /// Force the unpatched upstream prover instead of the version-specific memory optimizations.
    #[arg(long)]
    stock_prover: bool,

    /// Use the V2 low-memory profile (less RAM, with a throughput tradeoff).
    #[arg(long, conflicts_with = "stock_prover")]
    low_memory_prover: bool,
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

    // [1/8] Check Rust toolchains
    info!("[1/8] Checking Rust toolchains...");
    check_rust_toolchain().await?;
    info!("  Installing {STWO_NIGHTLY} (required by the pinned stwo stack)...");
    run_cmd("rustup", &["toolchain", "install", STWO_NIGHTLY]).await?;
    info!("");

    // [2/8] Clone/update proving-utils
    info!("[2/8] Setting up proving-utils...");
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

    // [3/8] Clone/update sequencer
    info!("[3/8] Setting up sequencer...");
    let sequencer_dir = deps_dir.join("sequencer");

    ensure_repo(
        &sequencer_dir,
        "https://github.com/starkware-libs/sequencer.git",
    )
    .await?;
    run_cmd_in(
        "git",
        &["fetch", "--quiet", "--tags", "origin"],
        &sequencer_dir,
    )
    .await?;

    info!("  Checking out {SEQUENCER_TAG}...");
    checkout_sequencer(&sequencer_dir, SEQUENCER_TAG).await?;

    // Apply macOS RLIMIT_AS patch
    apply_macos_patch(&sequencer_dir).await?;
    apply_low_memory_prover(
        deps_dir,
        &sequencer_dir,
        args.stock_prover,
        args.low_memory_prover,
    )
    .await?;
    info!("");

    // [4/8] Python venv
    info!("[4/8] Setting up Python virtual environment...");
    let requirements = sequencer_dir.join("scripts/requirements.txt");
    let venv_dir = ensure_venv(project_dir, &requirements).await?;
    info!("");

    // [5/8] Install Sierra compiler sidecar
    info!("[5/8] Installing starknet-sierra-compile...");
    install_sierra_compiler(&sequencer_dir, &config.compiler_tools_dir()).await?;
    info!("");

    // [6/8] Build stwo-run-and-prove
    if !args.skip_prover {
        info!("[6/8] Building stwo-run-and-prove...");
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
        info!("[6/8] Skipping stwo-run-and-prove build (--skip-prover)");
    }
    info!("");

    // [7/8] Build starknet_transaction_prover
    if !args.skip_runner {
        info!("[7/8] Building {RUNNER_PACKAGE} (requires {STWO_NIGHTLY} + venv)...");
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
            .env("CARGO_TOOLS_ROOT", config.compiler_tools_dir())
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
        info!("[7/8] Skipping {RUNNER_PACKAGE} build (--skip-runner)");
    }
    info!("");

    // [8/8] Copy bootloader program
    info!("[8/8] Locating bootloader program...");
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

    if let Some(sierra) = config.sierra_compiler_bin() {
        info!("  starknet-sierra-compile: {}", sierra.display());
    } else {
        bail!(
            "starknet-sierra-compile missing under {}",
            config.compiler_tools_dir().display()
        );
    }

    info!("");
    info!("=== Setup complete ===");
    info!("");
    info!("  Prover binary: {}", prover_bin.display());
    info!("  Runner binary: {}", runner_bin.display());
    info!(
        "  Compiler tools: {}",
        config.compiler_tools_dir().display()
    );
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
    if let Some(sierra) = config.sierra_compiler_bin() {
        info!("  starknet-sierra-compile: {}", sierra.display());
    } else {
        bail!(
            "starknet-sierra-compile missing under {} after extraction",
            config.compiler_tools_dir().display()
        );
    }

    info!("");
    info!("=== Setup complete (prebuilt {EXPECTED_DEPS_TAG}) ===");
    info!("");
    info!("  Prover binary: {}", prover_bin.display());
    info!("  Runner binary: {}", runner_bin.display());
    info!(
        "  Compiler tools: {}",
        config.compiler_tools_dir().display()
    );
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

/// Install the sequencer's Sierra compiler sidecar into a project-local cargo
/// tools root. Sequencer v0.14.4 resolves this path at runtime from
/// CARGO_TOOLS_ROOT instead of from a compile-time OUT_DIR.
async fn install_sierra_compiler(sequencer_dir: &Path, compiler_tools_dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(compiler_tools_dir).await?;
    let status = tokio::process::Command::new("bash")
        .args(["scripts/install_compiler_binaries.sh", "--sierra"])
        .current_dir(sequencer_dir)
        .env("CARGO_TOOLS_ROOT", compiler_tools_dir)
        .status()
        .await
        .wrap_err("failed to run sequencer compiler installer")?;
    if !status.success() {
        bail!("starknet-sierra-compile installer exited with {status}");
    }
    Ok(())
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

    // deps-v5+ tarballs ship the compiler sidecar in the versioned
    // CARGO_TOOLS_ROOT layout expected by the runner at runtime.
    let compiler_tools_src = bin_dir.join("compiler-tools");
    let compiler_tools_dst = deps_dir.join("compiler-tools");
    if compiler_tools_src.exists() {
        if compiler_tools_dst.exists() {
            tokio::fs::remove_dir_all(&compiler_tools_dst).await?;
        }
        tokio::fs::rename(&compiler_tools_src, &compiler_tools_dst).await?;
    }

    // Clean up the older deps-v4 shared_executables layout if a caller
    // explicitly extracts an older bundle with this newer CLI.
    let sierra_dir = release_dir.join("shared_executables");
    for src in [
        bin_dir.join("shared_executables/starknet-sierra-compile"),
        bin_dir.join("shared_executables/bin/starknet-sierra-compile"),
    ] {
        if src.exists() {
            tokio::fs::create_dir_all(&sierra_dir).await?;
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
        if let Some(sierra) = find_sierra_compiler_sync(&deps_dir.join("compiler-tools")) {
            tokio::fs::set_permissions(&sierra, std::fs::Permissions::from_mode(0o755)).await?;
        }
    }
    Ok(())
}

fn find_sierra_compiler_sync(root: &Path) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("starknet-sierra-compile-") {
            continue;
        }
        let binary = path.join("bin/starknet-sierra-compile");
        if binary.exists() {
            return Some(binary);
        }
    }
    None
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

#[derive(Debug, PartialEq, Eq)]
enum ProverMemoryMode {
    PatchedV1,
    PatchedV2,
}

fn prover_memory_mode(lock: &str) -> Result<ProverMemoryMode> {
    // V2 also depends on the V1 verifier: finding the old revisions anywhere in the lockfile
    // does not establish which prover is used. Select the actual privacy-prove package first.
    let packages: Vec<_> = lock
        .split("[[package]]")
        .filter(|package| {
            package
                .lines()
                .any(|line| line.trim() == "name = \"privacy-prove\"")
        })
        .collect();
    let [package] = packages.as_slice() else {
        bail!("expected exactly one privacy-prove package in sequencer Cargo.lock");
    };
    let source = package
        .lines()
        .find_map(|line| line.trim().strip_prefix("source = \"")?.strip_suffix('"'))
        .ok_or_else(|| eyre::eyre!("privacy-prove has no git source in sequencer Cargo.lock"))?;
    if source == "git+https://github.com/starkware-libs/proving?rev=e0035ffc8e08da270b8cb4a71f3708af70c2c775#e0035ffc8e08da270b8cb4a71f3708af70c2c775" {
        let prefix = "source = \"git+https://github.com/starkware-libs/proving?";
        let suffix = "#e0035ffc8e08da270b8cb4a71f3708af70c2c775\"";
        if lock.lines().any(|line| line.trim().starts_with(prefix) && !line.trim().ends_with(suffix)) {
            bail!("mixed proving revisions in sequencer Cargo.lock; review V2 patch compatibility");
        }
        return Ok(ProverMemoryMode::PatchedV2);
    }
    if source != "git+https://github.com/starkware-libs/proving-utils?tag=v0.14.3-rust-bump#3035dd00421daa541894297bd754db6e2787807b" {
        bail!("unsupported privacy-prove source {source}; review patch compatibility or use --stock-prover");
    }

    for (name, url, rev, _, _) in LOW_MEMORY_PROVER {
        let prefix = format!("source = \"git+{url}?");
        let suffix = format!("#{rev}\"");
        if !lock.lines().any(|line| {
            let line = line.trim();
            line.starts_with(&prefix) && line.ends_with(&suffix)
        }) {
            bail!("unsupported {name} revision for the V1 low-memory patches; regenerate the patches or use --stock-prover");
        }
    }
    Ok(ProverMemoryMode::PatchedV1)
}

/// Remove setup-generated manifest/lock changes before changing pins. A repo initialized over
/// cached build targets has no HEAD yet, so its first checkout needs no restoration.
async fn checkout_sequencer(sequencer_dir: &Path, revision: &str) -> Result<()> {
    let has_head = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(sequencer_dir)
        .output()
        .await
        .wrap_err("failed to inspect sequencer HEAD")?
        .status
        .success();
    if has_head {
        run_cmd_in(
            "git",
            &[
                "checkout",
                "--quiet",
                "HEAD",
                "--",
                "Cargo.toml",
                "Cargo.lock",
            ],
            sequencer_dir,
        )
        .await?;
    }
    run_cmd_in("git", &["checkout", "--quiet", revision], sequencer_dir).await
}

/// Apply the patch set for the supported prover, or restore upstream in stock mode.
async fn apply_low_memory_prover(
    deps_dir: &Path,
    sequencer_dir: &Path,
    stock: bool,
    enable_v2: bool,
) -> Result<()> {
    // Restore both files: a previous patched build rewrites git sources to local paths in the lock.
    run_cmd_in(
        "git",
        &["checkout", "--quiet", "--", "Cargo.toml", "Cargo.lock"],
        sequencer_dir,
    )
    .await?;
    if stock {
        info!("  Low-memory prover: skipped (--stock-prover)");
        return Ok(());
    }

    let lock = tokio::fs::read_to_string(sequencer_dir.join("Cargo.lock")).await?;
    let repos = match prover_memory_mode(&lock)? {
        ProverMemoryMode::PatchedV1 => LOW_MEMORY_PROVER,
        ProverMemoryMode::PatchedV2 if enable_v2 => LOW_MEMORY_PROVER_V2,
        ProverMemoryMode::PatchedV2 => {
            info!("  V2 prover: upstream (use --low-memory-prover for the lower-RAM profile)");
            return Ok(());
        }
    };

    info!("  Applying the low-memory prover patches...");
    let root = deps_dir.join("low-memory-prover");
    tokio::fs::create_dir_all(&root).await?;
    let mut section = String::from(
        "\n# snip36: low-memory prover (vendor/low-memory-prover), written by `snip36 setup`.\n",
    );
    for (name, url, rev, patch, crates) in repos {
        let dir = root.join(name);
        ensure_repo(&dir, &format!("{url}.git")).await?;
        run_cmd_in("git", &["fetch", "--quiet", "origin", rev], &dir).await?;
        // A clean tree at the pinned commit, then the patch: idempotent across re-runs.
        run_cmd_in("git", &["reset", "--quiet", "--hard", rev], &dir).await?;
        run_cmd_in("git", &["clean", "--quiet", "-fd"], &dir).await?;
        let patch_file = root.join(format!("{name}.patch"));
        tokio::fs::write(&patch_file, patch).await?;
        run_cmd_in("git", &["apply", &patch_file.to_string_lossy()], &dir)
            .await
            .wrap_err_with(|| format!("vendor/low-memory-prover/{name}.patch does not apply"))?;
        section.push_str(&format!("[patch.\"{url}\"]\n"));
        for (package, path) in *crates {
            section.push_str(&format!(
                "{package} = {{ path = {:?} }}\n",
                dir.join(path).to_string_lossy()
            ));
        }
        info!("    {name} @ {}: patched", &rev[..8]);
    }

    let manifest = sequencer_dir.join("Cargo.toml");
    let mut content = tokio::fs::read_to_string(&manifest).await?;
    content.push_str(&section);
    tokio::fs::write(&manifest, content).await?;
    Ok(())
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

#[cfg(test)]
mod memory_mode_tests {
    use super::*;

    fn package(name: &str, source: &str) -> String {
        format!("[[package]]\nname = \"{name}\"\nversion = \"1.0.0\"\nsource = \"{source}\"\n\n")
    }

    fn v1_lock() -> String {
        LOW_MEMORY_PROVER
            .iter()
            .map(|(name, url, rev, _, _)| {
                let package_name = if *name == "proving-utils" {
                    "privacy-prove"
                } else {
                    name
                };
                let query = if *name == "proving-utils" {
                    "tag=v0.14.3-rust-bump"
                } else {
                    "rev=pin"
                };
                package(package_name, &format!("git+{url}?{query}#{rev}"))
            })
            .collect()
    }

    fn v2_lock() -> String {
        // The real V2 lock contains all V1 patch revisions through its legacy verifier.
        let mut lock = v1_lock().replace(
            "name = \"privacy-prove\"",
            "name = \"privacy-circuit-verify\"",
        );
        lock.push_str(&package("privacy-prove", "git+https://github.com/starkware-libs/proving?rev=e0035ffc8e08da270b8cb4a71f3708af70c2c775#e0035ffc8e08da270b8cb4a71f3708af70c2c775"));
        lock
    }

    #[test]
    fn v2_with_legacy_verifier_selects_only_v2_patches() {
        assert_eq!(
            prover_memory_mode(&v2_lock()).unwrap(),
            ProverMemoryMode::PatchedV2
        );
        assert_eq!(
            prover_memory_mode(&v1_lock()).unwrap(),
            ProverMemoryMode::PatchedV1
        );
    }

    #[test]
    fn unknown_missing_or_ambiguous_provers_are_rejected() {
        assert!(prover_memory_mode("").is_err());
        assert!(prover_memory_mode(&v2_lock().replace("e0035ffc", "ffffffff")).is_err());
        assert!(prover_memory_mode(&(v1_lock() + &v2_lock())).is_err());
        assert!(prover_memory_mode("[[package]]\nname = \"privacy-prove\"\n").is_err());
    }

    #[test]
    fn v2_rejects_a_mixed_monorepo_even_with_the_supported_privacy_prover() {
        let mixed = v2_lock()
            + &package(
                "circuit-prover",
                "git+https://github.com/starkware-libs/proving?rev=unknown#unknown",
            );
        assert!(prover_memory_mode(&mixed).is_err());
        assert_eq!(LOW_MEMORY_PROVER_V2.len(), 1);
        let packages = LOW_MEMORY_PROVER_V2[0].4;
        for name in [
            "privacy-prove",
            "circuits",
            "circuit-prover",
            "stwo",
            "privacy_common",
        ] {
            assert!(packages.iter().any(|(package, _)| *package == name));
        }
    }

    #[test]
    fn v1_requires_each_patch_source_at_the_supported_revision() {
        for (_, url, rev, _, _) in LOW_MEMORY_PROVER {
            assert!(prover_memory_mode(&v1_lock().replace(rev, "unknown")).is_err());
            assert!(
                prover_memory_mode(&v1_lock().replace(url, "https://example.invalid/other"))
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn stock_setup_removes_stale_manifest_and_lock_patches_without_cloning() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let sequencer = tmp.path().join("sequencer");
        tokio::fs::create_dir(&sequencer).await?;
        let manifest = "[workspace]\nmembers = []\n";
        let lock = v2_lock();
        tokio::fs::write(sequencer.join("Cargo.toml"), manifest).await?;
        tokio::fs::write(sequencer.join("Cargo.lock"), &lock).await?;
        run_cmd_in("git", &["init", "--quiet"], &sequencer).await?;
        run_cmd_in("git", &["add", "Cargo.toml", "Cargo.lock"], &sequencer).await?;
        run_cmd_in(
            "git",
            &[
                "-c",
                "user.name=Setup Test",
                "-c",
                "user.email=setup@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
            &sequencer,
        )
        .await?;
        tokio::fs::write(sequencer.join("Cargo.toml"), "stale patch section").await?;
        tokio::fs::write(
            sequencer.join("Cargo.lock"),
            "stale local path dependencies",
        )
        .await?;

        apply_low_memory_prover(tmp.path(), &sequencer, true, false).await?;
        // The ordinary V2 default must also restore upstream, without any clone.
        apply_low_memory_prover(tmp.path(), &sequencer, false, false).await?;
        assert_eq!(
            tokio::fs::read_to_string(sequencer.join("Cargo.toml")).await?,
            manifest
        );
        assert_eq!(
            tokio::fs::read_to_string(sequencer.join("Cargo.lock")).await?,
            lock
        );
        assert!(!tmp.path().join("low-memory-prover").exists());
        Ok(())
    }

    #[tokio::test]
    async fn upgrade_dirty_v1_checkout_and_initialize_cached_repo() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let sequencer = tmp.path().join("sequencer");
        tokio::fs::create_dir(&sequencer).await?;
        run_cmd_in("git", &["init", "--quiet"], &sequencer).await?;
        let commit = [
            "-c",
            "user.name=Setup Test",
            "-c",
            "user.email=setup@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ];
        tokio::fs::write(sequencer.join("Cargo.toml"), "old manifest").await?;
        tokio::fs::write(sequencer.join("Cargo.lock"), v1_lock()).await?;
        tokio::fs::write(sequencer.join("notes.txt"), "original notes").await?;
        run_cmd_in("git", &["add", "."], &sequencer).await?;
        run_cmd_in("git", &commit, &sequencer).await?;
        run_cmd_in("git", &["tag", "old-pin"], &sequencer).await?;
        tokio::fs::write(sequencer.join("Cargo.toml"), "new manifest").await?;
        tokio::fs::write(sequencer.join("Cargo.lock"), v2_lock()).await?;
        run_cmd_in("git", &["add", "."], &sequencer).await?;
        run_cmd_in("git", &commit, &sequencer).await?;
        run_cmd_in("git", &["tag", "new-pin"], &sequencer).await?;
        run_cmd_in("git", &["checkout", "--quiet", "old-pin"], &sequencer).await?;
        tokio::fs::write(
            sequencer.join("Cargo.toml"),
            "old manifest with local patches",
        )
        .await?;
        tokio::fs::write(sequencer.join("Cargo.lock"), "local path dependencies").await?;
        tokio::fs::write(sequencer.join("notes.txt"), "keep these notes").await?;
        let rejected = tokio::process::Command::new("git")
            .args(["checkout", "--quiet", "new-pin"])
            .current_dir(&sequencer)
            .output()
            .await?;
        assert!(
            !rejected.status.success(),
            "fixture must reproduce the upgrade failure"
        );

        checkout_sequencer(&sequencer, "new-pin").await?;
        apply_low_memory_prover(tmp.path(), &sequencer, true, false).await?;
        assert_eq!(
            tokio::fs::read_to_string(sequencer.join("Cargo.toml")).await?,
            "new manifest"
        );
        assert_eq!(
            tokio::fs::read_to_string(sequencer.join("Cargo.lock")).await?,
            v2_lock()
        );
        assert_eq!(
            tokio::fs::read_to_string(sequencer.join("notes.txt")).await?,
            "keep these notes"
        );
        assert!(!tmp.path().join("low-memory-prover").exists());

        // CI may restore target/ into an initialized repository without a checked-out HEAD.
        let fresh = tmp.path().join("cached-repo");
        tokio::fs::create_dir(&fresh).await?;
        run_cmd_in("git", &["init", "--quiet"], &fresh).await?;
        run_cmd_in(
            "git",
            &["fetch", "--quiet", sequencer.to_str().unwrap(), "new-pin"],
            &fresh,
        )
        .await?;
        checkout_sequencer(&fresh, "FETCH_HEAD").await?;
        assert_eq!(
            tokio::fs::read_to_string(fresh.join("Cargo.lock")).await?,
            v2_lock()
        );
        Ok(())
    }
}
