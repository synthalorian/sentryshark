use axum::{
    routing::{post, get},
    Router,
    http::StatusCode,
    response::Json,
    extract::ConnectInfo,
};
use std::net::SocketAddr;
use std::sync::Arc;
use serde::Serialize;
use tracing::{info, instrument};

use sentryshark::config::AppConfig;
use sentryshark::db::Database;
use sentryshark::metrics::Metrics;
use sentryshark::rate_limit::{extract_client_key, RateLimiter};
use sentryshark::shutdown::{wait_for_shutdown, ShutdownHandle};
use sentryshark::AppState;

#[derive(Serialize)]
struct HealthStatus {
    status: String,
    version: String,
    database: String,
    config_loaded: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Arc::new(AppConfig::load()?);
    let logging_config = config.logging_config();

    if logging_config.json_format {
        tracing_subscriber::fmt()
            .json()
            .with_target(true)
            .init();
    } else {
        tracing_subscriber::fmt::init();
    }

    let db_path = config.database_config().path.clone();
    let database = Arc::new(Database::new(&db_path)?);
    let metrics = Arc::new(Metrics::new());
    let rate_limiter = Arc::new(RateLimiter::new(60, 60));
    let shutdown = ShutdownHandle::new();

    let state = AppState {
        config,
        database,
        metrics,
        rate_limiter,
    };

    let dashboard_enabled = state.config.dashboard_config().enabled;

    let mut app = Router::new()
        .route("/webhook/github", post(github_webhook))
        .route("/webhook/gitlab", post(gitlab_webhook))
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_handler));

    if dashboard_enabled {
        app = app
            .route("/dashboard", get(sentryshark::dashboard::dashboard_handler))
            .route("/dashboard/stats", get(sentryshark::dashboard::stats_api_handler))
            .route("/dashboard/api/search", get(sentryshark::dashboard::search_api_handler));
        info!("\u{1f4ca} Dashboard enabled at /dashboard");
    }

    let app = app.with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    info!("\u{1f988} SentryShark v1.0.0 listening on {}", listener.local_addr()?);

    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        wait_for_shutdown().await;
        shutdown_clone.shutdown();
    });

    axum::serve(listener, app).await?;
    Ok(())
}

#[instrument(skip(state, headers, body), fields(provider = "github"))]
async fn github_webhook(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    let client_key = extract_client_key(Some(addr));
    state.metrics.record_webhook_received();

    if !state.rate_limiter.is_allowed(&client_key).await {
        state.metrics.record_webhook_rate_limited();
        tracing::warn!("Rate limit exceeded for {}", client_key);
        return StatusCode::TOO_MANY_REQUESTS;
    }

    sentryshark::github::webhook_handler(state, headers, body).await
}

#[instrument(skip(state, headers, body), fields(provider = "gitlab"))]
async fn gitlab_webhook(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    let client_key = extract_client_key(Some(addr));
    state.metrics.record_webhook_received();

    if !state.rate_limiter.is_allowed(&client_key).await {
        state.metrics.record_webhook_rate_limited();
        tracing::warn!("Rate limit exceeded for {}", client_key);
        return StatusCode::TOO_MANY_REQUESTS;
    }

    sentryshark::gitlab::webhook_handler(state, headers, body).await
}

async fn health_check(axum::extract::State(state): axum::extract::State<AppState>) -> Json<HealthStatus> {
    let db_status = if state.database.get_stats().await.is_ok() {
        "connected"
    } else {
        "error"
    };

    Json(HealthStatus {
        status: "healthy".to_string(),
        version: "1.0.0".to_string(),
        database: db_status.to_string(),
        config_loaded: true,
    })
}

async fn metrics_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<(axum::http::header::HeaderMap, String), StatusCode> {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        "text/plain; charset=utf-8".parse().unwrap(),
    );

    let body = state.metrics.render_prometheus().await;
    Ok((headers, body))
}
