use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
    routing::{post, get},
};
use std::sync::Arc;
use tower::util::ServiceExt;

use sentryshark::config::{
    AppConfig, GitHubConfig, GitLabConfig, LlmConfig, ReviewConfig,
    DiffFilterConfig, BatchingConfig, DatabaseConfig, DashboardConfig,
    AutoApproveConfig, RetryConfig, QueueConfig, CacheConfig,
};
use sentryshark::db::Database;
use sentryshark::metrics::Metrics;
use sentryshark::rate_limit::RateLimiter;
use sentryshark::AppState;

/// End-to-end integration test simulating a real GitHub webhook payload flow.
/// This test verifies the complete webhook handling pipeline from receipt
/// through signature verification to job queueing.
fn create_e2e_app() -> Router {
    let config = AppConfig {
        server: sentryshark::config::ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
        },
        github: Some(GitHubConfig {
            webhook_secret: "e2e-test-secret".to_string(),
            app_id: "e2e-test-app-id".to_string(),
            private_key_path: "/tmp/e2e-test-key.pem".to_string(),
            use_app_auth: false,
            installation_id: Some(12345678),
        }),
        gitlab: Some(GitLabConfig {
            webhook_secret: "e2e-gitlab-secret".to_string(),
            access_token: "e2e-gitlab-token".to_string(),
            ci_cd_enabled: false,
            base_url: "https://gitlab.com".to_string(),
        }),
        llm: LlmConfig {
            provider: "llamacpp".to_string(),
            base_url: "http://localhost:8080".to_string(),
            model: "codellama-34b".to_string(),
            max_tokens: 4096,
            temperature: 0.1,
        },
        review: Some(ReviewConfig {
            security: true,
            style: true,
            performance: true,
            correctness: true,
            maintainability: true,
            inline_comments: true,
            summary_comment: true,
            template: None,
        }),
        diff_filter: Some(DiffFilterConfig {
            enabled: true,
            lockfile_patterns: vec!["Cargo.lock".to_string(), "*.lock".to_string()],
            generated_patterns: vec!["*.min.js".to_string(), "dist/".to_string()],
            include_patterns: vec![],
            exclude_patterns: vec![],
        }),
        batching: Some(BatchingConfig {
            enabled: false,
            timeout_seconds: 30,
            max_size: 10,
        }),
        database: Some(DatabaseConfig::default()),
        dashboard: Some(DashboardConfig::default()),
        auto_approve: Some(AutoApproveConfig::default()),
        retry: Some(RetryConfig::default()),
        queue: Some(QueueConfig::default()),
        cache: Some(CacheConfig::default()),
        rules: Some(sentryshark::config::RulesConfig::default()),
        logging: Some(sentryshark::config::LoggingConfig::default()),
    };

    let database = Arc::new(Database::new(":memory:").unwrap());
    let metrics = Arc::new(Metrics::new());
    let rate_limiter = Arc::new(RateLimiter::new(60, 60));

    let state = AppState {
        config: Arc::new(config),
        database,
        metrics,
        rate_limiter,
    };

    Router::new()
        .route("/webhook/github", post(sentryshark::github::webhook_handler))
        .route("/webhook/gitlab", post(sentryshark::gitlab::webhook_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/dashboard", get(sentryshark::dashboard::dashboard_handler))
        .route("/dashboard/stats", get(sentryshark::dashboard::stats_api_handler))
        .route("/dashboard/api/search", get(sentryshark::dashboard::search_api_handler))
        .with_state(state)
}

async fn health_handler() -> StatusCode {
    StatusCode::OK
}

async fn metrics_handler(axum::extract::State(state): axum::extract::State<AppState>) -> String {
    state.metrics.render_prometheus().await
}

fn compute_github_signature(secret: &str, body: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => panic!("HMAC initialization failed"),
    };
    mac.update(body.as_bytes());
    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    format!("sha256={}", hex::encode(code_bytes))
}

/// Simulates a real GitHub "pull_request opened" webhook payload
fn real_github_pr_payload() -> String {
    r#"{
        "action": "opened",
        "number": 42,
        "pull_request": {
            "number": 42,
            "title": "Add user authentication middleware",
            "body": "This PR adds JWT-based authentication middleware to protect API endpoints.\n\n## Changes\n- Add `auth.rs` with JWT validation\n- Update `main.rs` to apply middleware to protected routes\n- Add configuration for JWT secret",
            "head": {
                "ref": "feature/auth-middleware",
                "sha": "a1b2c3d4e5f6789012345678901234567890abcd",
                "repo": {
                    "id": 123456789,
                    "name": "sentryshark",
                    "full_name": "synthalorian/sentryshark",
                    "clone_url": "https://github.com/synthalorian/sentryshark.git",
                    "html_url": "https://github.com/synthalorian/sentryshark"
                }
            },
            "base": {
                "ref": "main",
                "sha": "f0e1d2c3b4a59687766554433221100998877665",
                "repo": {
                    "id": 123456789,
                    "name": "sentryshark",
                    "full_name": "synthalorian/sentryshark",
                    "clone_url": "https://github.com/synthalorian/sentryshark.git"
                }
            },
            "state": "open",
            "locked": false,
            "user": {
                "login": "synthalorian",
                "id": 98765432
            },
            "created_at": "2024-01-15T10:30:00Z",
            "updated_at": "2024-01-15T10:30:00Z"
        },
        "repository": {
            "id": 123456789,
            "name": "sentryshark",
            "full_name": "synthalorian/sentryshark",
            "private": false,
            "clone_url": "https://github.com/synthalorian/sentryshark.git",
            "html_url": "https://github.com/synthalorian/sentryshark",
            "owner": {
                "login": "synthalorian",
                "id": 98765432
            }
        },
        "sender": {
            "login": "synthalorian",
            "id": 98765432
        }
    }"#.to_string()
}

#[tokio::test]
async fn test_e2e_github_webhook_full_pipeline() {
    let app = create_e2e_app();
    let body = real_github_pr_payload();
    let signature = compute_github_signature("e2e-test-secret", &body);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header("x-hub-signature-256", signature)
                .header("content-type", "application/json")
                .header("x-github-event", "pull_request")
                .header("x-github-delivery", "72d3162e-cc78-11e3-81ab-4c9367dc0958")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 200 OK after queueing the review job
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_e2e_health_check_detailed() {
    let app = create_e2e_app();

    let response = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_e2e_metrics_endpoint_prometheus() {
    let app = create_e2e_app();

    let response = app
        .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    // Verify Prometheus format
    assert!(body_str.contains("# TYPE"));
    assert!(body_str.contains("sentryshark_reviews_total"));
    assert!(body_str.contains("sentryshark_webhooks_received"));
}

#[tokio::test]
async fn test_e2e_dashboard_renders() {
    let app = create_e2e_app();

    let response = app
        .oneshot(Request::builder().uri("/dashboard").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    assert!(body_str.contains("SentryShark Dashboard"));
}

#[tokio::test]
async fn test_e2e_dashboard_stats_api() {
    let app = create_e2e_app();

    let response = app
        .oneshot(Request::builder().uri("/dashboard/stats").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    // Should return valid JSON
    let json: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert!(json["total_reviews"].is_number());
    assert!(json["approved"].is_number());
}

#[tokio::test]
async fn test_e2e_rate_limiting() {
    let app = create_e2e_app();

    // The rate limiter is set to 60 requests per minute, so we shouldn't hit it
    // with just a few requests, but let's verify the endpoint works
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn test_e2e_github_webhook_synchronize_action() {
    let app = create_e2e_app();

    let body = r#"{
        "action": "synchronize",
        "pull_request": {
            "number": 42,
            "title": "Update auth middleware",
            "body": "Fixed review feedback",
            "head": {
                "ref": "feature/auth-middleware",
                "sha": "b2c3d4e5f6a7890123456789012345678901bcde",
                "repo": {
                    "clone_url": "https://github.com/synthalorian/sentryshark.git"
                }
            },
            "base": {
                "ref": "main",
                "sha": "f0e1d2c3b4a59687766554433221100998877665"
            }
        },
        "repository": {
            "full_name": "synthalorian/sentryshark",
            "clone_url": "https://github.com/synthalorian/sentryshark.git"
        }
    }"#;

    let signature = compute_github_signature("e2e-test-secret", body);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header("x-hub-signature-256", signature)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_e2e_gitlab_webhook_full_pipeline() {
    let app = create_e2e_app();

    let body = r#"{
        "object_kind": "merge_request",
        "project": {
            "id": 123456,
            "path_with_namespace": "synthalorian/sentryshark",
            "git_http_url": "https://gitlab.com/synthalorian/sentryshark.git"
        },
        "object_attributes": {
            "iid": 42,
            "title": "Add authentication middleware",
            "description": "Implements JWT-based authentication",
            "source_branch": "feature/auth-middleware",
            "target_branch": "main",
            "last_commit": {
                "id": "a1b2c3d4e5f6789012345678901234567890abcd"
            },
            "state": "opened",
            "action": "open"
        }
    }"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/gitlab")
                .header("x-gitlab-token", "e2e-gitlab-secret")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_e2e_invalid_github_signature_blocks_request() {
    let app = create_e2e_app();
    let body = real_github_pr_payload();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header("x-hub-signature-256", "sha256=invalidsignature123")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_e2e_config_validation() {
    let config = AppConfig {
        server: sentryshark::config::ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
        },
        github: Some(GitHubConfig {
            webhook_secret: "valid-secret".to_string(),
            app_id: "valid-app-id".to_string(),
            private_key_path: "/tmp/valid.pem".to_string(),
            use_app_auth: false,
            installation_id: None,
        }),
        gitlab: None,
        llm: LlmConfig {
            provider: "llamacpp".to_string(),
            base_url: "http://localhost:8080".to_string(),
            model: "codellama".to_string(),
            max_tokens: 4096,
            temperature: 0.1,
        },
        review: Some(ReviewConfig::default()),
        diff_filter: Some(DiffFilterConfig::default()),
        batching: Some(BatchingConfig::default()),
        database: Some(DatabaseConfig::default()),
        dashboard: Some(DashboardConfig::default()),
        ..Default::default()
    };

    assert!(config.validate().is_ok());
}
