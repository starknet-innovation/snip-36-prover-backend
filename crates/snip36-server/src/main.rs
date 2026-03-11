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

    let state = Arc::new(AppState::new(config));

    // CORS: allow all origins (playground demo)
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = routes::api_router().with_state(state).layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    info!("Listening on 0.0.0.0:8080");
    axum::serve(listener, app).await?;

    Ok(())
}
