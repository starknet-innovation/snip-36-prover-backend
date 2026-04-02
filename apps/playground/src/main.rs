//! Full SNIP-36 playground server.
//!
//! Composes the generic SDK routes with Counter and CoinFlip example routes.

use std::sync::Arc;

use snip36_core::config::Config;
use snip36_server::AppState;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env(None)?;
    info!(rpc_url = %config.rpc_url, "Starting SNIP-36 playground");

    ensure_sncast_account(&config).await;

    let state = Arc::new(AppState::new(config));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Compose generic SDK routes + app-specific routes
    let app = snip36_server::routes::generic_routes()
        .with_state(state.clone())
        .merge(snip36_counter::routes::counter_routes().with_state(state.clone()))
        .merge(snip36_coinflip::routes::coinflip_routes(state))
        .layer(cors);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8090".into());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// Import the master account into sncast so deploy/fund routes work on fresh installs.
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
