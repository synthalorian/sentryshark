use axum::{
    routing::{post, get},
    Router,
    http::StatusCode,
};
use std::sync::Arc;
use tracing::info;

use sentryshark::config::AppConfig;
use sentryshark::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Arc::new(AppConfig::load()?);
    let state = AppState { config };

    let app = Router::new()
        .route("/webhook/github", post(sentryshark::github::webhook_handler))
        .route("/webhook/gitlab", post(sentryshark::gitlab::webhook_handler))
        .route("/health", get(health_check))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("🦈 SentryShark listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_check() -> StatusCode {
    StatusCode::OK
}
