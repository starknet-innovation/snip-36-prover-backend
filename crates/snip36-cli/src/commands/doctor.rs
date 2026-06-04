use std::path::Path;

use clap::Args;
use color_eyre::eyre::Result;
use tracing::{error, info, warn};

use snip36_core::Config;

use super::setup::{venv_is_healthy, EXPECTED_DEPS_TAG};

#[derive(Args)]
pub struct DoctorArgs {}

struct Checks {
    passed: u32,
    failed: u32,
}

impl Checks {
    fn pass(&mut self, name: &str, detail: &str) {
        self.passed += 1;
        if detail.is_empty() {
            info!("  PASS: {name}");
        } else {
            info!("  PASS: {name} ({detail})");
        }
    }

    fn fail(&mut self, name: &str, detail: &str) {
        self.failed += 1;
        if detail.is_empty() {
            error!("  FAIL: {name}");
        } else {
            error!("  FAIL: {name} -- {detail}");
        }
    }
}

/// Offline validation of the local proving stack — no RPC, no keys. Checks
/// the binaries, bootloader, and Python venv that `prove` will need, with
/// actionable fixes, before a failure surfaces deep in the pipeline.
pub async fn run(_args: DoctorArgs, env_file: Option<&Path>) -> Result<()> {
    // Doctor must work before .env exists; fall back to cwd-based defaults
    // (mirrors `setup`).
    let config = Config::from_env(env_file).unwrap_or_else(|_| {
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

    info!("=== SNIP-36 Doctor (offline stack check) ===");
    info!("  Project dir: {}", config.project_dir.display());
    info!(
        "  Host:        {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    info!("");

    let mut checks = Checks {
        passed: 0,
        failed: 0,
    };

    // Deps provenance (stamp written by `setup --prebuilt` / download-deps.sh;
    // source-built deps have no stamp).
    match tokio::fs::read_to_string(config.deps_dir.join(".deps-version")).await {
        Ok(tag) => {
            let tag = tag.trim().to_string();
            if tag == EXPECTED_DEPS_TAG {
                checks.pass("deps release", &tag);
            } else {
                // Not a hard failure — the binaries may still be compatible.
                warn!(
                    "  WARN: deps release is '{tag}' but this snip36 build expects \
                     '{EXPECTED_DEPS_TAG}' — run `snip36 setup --prebuilt` to refresh"
                );
            }
        }
        Err(_) => info!("  INFO: no deps/.deps-version stamp (source-built deps)"),
    }

    // Prover + runner: present AND runnable (`--help` catches a binary for
    // the wrong architecture or a corrupted download).
    check_binary(&mut checks, "stwo-run-and-prove", &config.prover_bin()).await;
    check_binary(&mut checks, "virtual-OS runner", &config.runner_bin()).await;

    // Sierra compiler (invoked by the runner; no --help contract, so presence
    // + executable bit only).
    let sierra = config
        .deps_dir
        .join("sequencer/target/release/shared_executables/starknet-sierra-compile");
    if sierra.exists() {
        checks.pass("starknet-sierra-compile", "");
    } else {
        checks.fail(
            "starknet-sierra-compile",
            &format!(
                "missing at {} — run `snip36 setup --prebuilt`",
                sierra.display()
            ),
        );
    }

    // Bootloader program
    let bootloader = config.bootloader_program();
    if bootloader.exists() {
        checks.pass("bootloader_program.json", "");
    } else {
        checks.fail(
            "bootloader_program.json",
            &format!(
                "missing at {} — run `snip36 setup --prebuilt`",
                bootloader.display()
            ),
        );
    }

    // Prover parameter templates (prove program / prove pie)
    if config.prover_params().exists() {
        checks.pass("prover params template", "");
    } else {
        checks.fail(
            "prover params template",
            &format!(
                "missing at {} — run from the repo root or set SNIP36_PROJECT_DIR",
                config.prover_params().display()
            ),
        );
    }

    // Python venv: console-script shebangs embed absolute paths, so probe pip
    // rather than trusting file existence (catches a moved project directory
    // or an upgraded/removed base interpreter).
    let venv_dir = config.project_dir.join("sequencer_venv");
    if !venv_dir.join("bin/cairo-compile").exists() {
        checks.fail(
            "cairo-compile venv",
            "sequencer_venv/bin/cairo-compile missing — run `snip36 setup --prebuilt`",
        );
    } else if venv_is_healthy(&venv_dir).await {
        checks.pass("cairo-compile venv", "");
    } else {
        checks.fail(
            "cairo-compile venv",
            "venv interpreter is stale (project moved or python upgraded/removed) — \
             re-run `snip36 setup --prebuilt` to recreate it",
        );
    }

    let Checks { passed, failed } = checks;
    info!("");
    info!("==========================================");
    info!("  Passed: {passed}");
    info!("  Failed: {failed}");
    info!("==========================================");

    if failed > 0 {
        info!("  RESULT: {failed} CHECK(S) FAILED");
        std::process::exit(1);
    }
    info!("  RESULT: PROVING STACK READY");
    Ok(())
}

async fn check_binary(checks: &mut Checks, name: &str, path: &Path) {
    if !path.exists() {
        checks.fail(
            name,
            &format!(
                "missing at {} — run `snip36 setup --prebuilt`",
                path.display()
            ),
        );
        return;
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
        checks.pass(name, &path.display().to_string());
    } else {
        checks.fail(
            name,
            &format!(
                "{} exists but `--help` failed (wrong architecture or corrupted \
                 download) — re-run `snip36 setup --prebuilt`",
                path.display()
            ),
        );
    }
}
