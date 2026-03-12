use std::path::PathBuf;

use clap::Args;
use color_eyre::eyre::{bail, Result, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info};

use snip36_core::Config;

#[derive(Args)]
pub struct ExtractArgs {
    /// Output path for the virtual OS program JSON
    #[arg(long, default_value = "output/virtual_os_program.json")]
    output: PathBuf,

    /// Use a pre-built virtual OS program instead of extracting
    #[arg(long)]
    program: Option<PathBuf>,
}

pub async fn run(args: ExtractArgs, env_file: Option<&std::path::Path>) -> Result<()> {
    if let Some(parent) = args.output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // If pre-built program provided, just copy it
    if let Some(program_path) = &args.program {
        if !program_path.exists() {
            bail!("file not found: {}", program_path.display());
        }
        info!("Using pre-built program: {}", program_path.display());
        tokio::fs::copy(program_path, &args.output).await?;
        info!("Copied to: {}", args.output.display());
        return Ok(());
    }

    let config = Config::from_env(env_file)?;
    let sequencer_dir = config.deps_dir.join("sequencer");

    if !sequencer_dir.exists() {
        bail!(
            "{} not found. Run `snip36 setup` and clone the sequencer repo, \
             or use --program <path> to provide a pre-built program.",
            sequencer_dir.display()
        );
    }

    info!("=== Extracting Virtual OS Program ===");
    info!("");

    // [1/2] Build extractor
    info!("[1/2] Building extractor...");
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner().template("{spinner} {msg}")?,
    );
    pb.set_message("Building virtual-os-extractor...");
    pb.enable_steady_tick(std::time::Duration::from_millis(200));

    let extractor_manifest = config.project_dir.join("extractor/Cargo.toml");
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.args([
        "build",
        "--release",
        "--manifest-path",
        &extractor_manifest.to_string_lossy(),
    ])
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().wrap_err("failed to build extractor")?;

    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "extractor", "{line}");
            }
        });
    }

    let status = child.wait().await?;
    pb.finish_and_clear();

    if !status.success() {
        bail!("extractor build failed");
    }
    info!("");

    // [2/2] Run extractor
    info!("[2/2] Extracting virtual OS program...");

    // Try both possible binary locations
    let extractor_bin = config
        .project_dir
        .join("extractor/target/release/virtual-os-extractor");
    let extractor_bin = if extractor_bin.exists() {
        extractor_bin
    } else {
        config
            .project_dir
            .join("target/release/virtual-os-extractor")
    };

    if !extractor_bin.exists() {
        bail!("extractor binary not found at {}", extractor_bin.display());
    }

    let status = tokio::process::Command::new(&extractor_bin)
        .arg(&args.output)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status()
        .await
        .wrap_err("failed to run extractor")?;

    if !status.success() {
        bail!("extractor exited with {status}");
    }

    info!("");
    info!("Virtual OS program: {}", args.output.display());

    Ok(())
}
