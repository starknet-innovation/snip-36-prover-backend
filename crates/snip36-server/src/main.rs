mod routes;
mod state;

use std::sync::Arc;

use snip36_core::config::Config;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    // Set up tracing with RUST_LOG env filter
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load configuration from environment / .env
    let config = Config::from_env(None)?;
    info!(rpc_url = %config.rpc_url, "Starting SNIP-36 server");

    // Import the sncast account so deploy/fund routes work on fresh installs
    ensure_sncast_account(&config).await;

    let state = Arc::new(AppState::new(config));

    // CORS: allow all origins (playground demo)
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = routes::api_router().with_state(state).layer(cors);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8090".into());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Import the master account into sncast so deploy/fund routes can use it.
///
/// This is idempotent — if the account already exists, sncast returns an error
/// which we silently ignore.
async fn ensure_sncast_account(config: &Config) {
    let account_name = config.sncast_account();
    info!(account = %account_name, "Importing sncast account");

    let result = tokio::process::Command::new("sncast")
        .args([
            "account",
            "import",
            "--name",
            &account_name,
            "--address",
            &config.account_address,
            "--private-key",
            &config.private_key,
            "--type",
            "oz",
            "--url",
            &config.rpc_url,
        ])
        .output()
        .await;

    match result {
        Ok(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            if output.status.success() {
                info!("sncast account '{account_name}' imported");
            } else if combined.contains("already exists") {
                info!("sncast account '{account_name}' already exists");
            } else {
                tracing::warn!("sncast account import failed: {combined}");
            }
        }
        Err(e) => {
            tracing::warn!("Could not run sncast to import account: {e}");
        }
    }
}
