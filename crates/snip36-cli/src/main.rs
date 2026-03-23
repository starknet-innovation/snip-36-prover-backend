use std::path::PathBuf;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use tracing_subscriber::EnvFilter;

mod commands;

#[derive(Parser)]
#[command(name = "snip36", about = "Unified CLI for SNIP-36 virtual block proving")]
struct Cli {
    /// Path to .env file for configuration
    #[arg(long, global = true)]
    env_file: Option<PathBuf>,

    /// Enable verbose (debug) logging
    #[arg(long, short, global = true)]
    verbose: bool,

    /// Suppress all output except errors
    #[arg(long, short, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run virtual OS + stwo prover
    Prove(commands::prove::ProveArgs),
    /// Sign and submit proof via RPC
    Submit(commands::submit::SubmitArgs),
    /// Deploy contracts via sncast
    Deploy(commands::deploy::DeployArgs),
    /// Transfer STRK from master account
    Fund(commands::fund::FundArgs),
    /// Extract virtual OS program
    Extract(commands::extract::ExtractArgs),
    /// CI health check
    Health(commands::health::HealthArgs),
    /// Environment setup
    Setup(commands::setup::SetupArgs),
    /// Full end-to-end test
    E2e(commands::e2e::E2eArgs),
    /// E2E test for L2→L1 messages (raw_messages.json)
    E2eMessages(commands::e2e_messages::E2eMessagesArgs),
    /// E2E coin flip example (provable off-chain game)
    E2eCoinflip(commands::e2e_coinflip::E2eCoinflipArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let filter = if cli.quiet {
        EnvFilter::new("error")
    } else if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let env_file = cli.env_file.as_deref();

    match cli.command {
        Commands::Prove(args) => commands::prove::run(args, env_file).await,
        Commands::Submit(args) => commands::submit::run(args, env_file).await,
        Commands::Deploy(args) => commands::deploy::run(args, env_file).await,
        Commands::Fund(args) => commands::fund::run(args, env_file).await,
        Commands::Extract(args) => commands::extract::run(args, env_file).await,
        Commands::Health(args) => commands::health::run(args, env_file).await,
        Commands::Setup(args) => commands::setup::run(args, env_file).await,
        Commands::E2e(args) => commands::e2e::run(args, env_file).await,
        Commands::E2eMessages(args) => commands::e2e_messages::run(args, env_file).await,
        Commands::E2eCoinflip(args) => commands::e2e_coinflip::run(args, env_file).await,
    }
}
