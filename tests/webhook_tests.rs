use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
    routing::{post, get},
};
use std::sync::Arc;
use tower::util::ServiceExt;

use sentryshark::config::{AppConfig, GitHubConfig, GitLabConfig, LlmConfig};
use sentryshark::AppState;

fn create_test_app() -> Router {
    let config = AppConfig {
        server: sentryshark::config::ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
        },
        github: Some(GitHubConfig {
            webhook_secret: "github-secret".to_string(),
            app_id: "test-app-id".to_string(),
            private_key_path: "/tmp/test-key.pem".to_string(),
        }),
        gitlab: Some(GitLabConfig {
            webhook_secret: "gitlab-secret".to_string(),
            access_token: "test-token".to_string(),
        }),
        llm: LlmConfig {
            provider: "test".to_string(),
            base_url: "http://localhost:8080".to_string(),
            model: "test-model".to_string(),
            max_tokens: 100,
            temperature: 0.1,
        },
    };

    let state = AppState {
        config: Arc::new(config),
    };

    Router::new()
        .route("/webhook/github", post(sentryshark::github::webhook_handler))
        .route("/webhook/gitlab", post(sentryshark::gitlab::webhook_handler))
        .route("/health", get(|| async { StatusCode::OK }))
        .with_state(state)
}

#[tokio::test]
async fn test_health_check() {
    let app = create_test_app();

    let response = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_github_webhook_valid_signature() {
    let app = create_test_app();

    let body = r#"{
        "action": "opened",
        "pull_request": {
            "number": 1,
            "title": "Test PR",
            "body": "Test body",
            "head": {
                "ref": "feature-branch",
                "sha": "abc123",
                "repo": {
                    "clone_url": "https://github.com/test/repo.git"
                }
            },
            "base": {
                "ref": "main",
                "sha": "def456"
            }
        },
        "repository": {
            "full_name": "test/repo",
            "clone_url": "https://github.com/test/repo.git"
        }
    }"#;

    let signature = compute_github_signature("github-secret", body);

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
async fn test_github_webhook_invalid_signature() {
    let app = create_test_app();

    let body = r#"{"action": "opened", "repository": {"full_name": "test/repo", "clone_url": "https://github.com/test/repo.git"}}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header("x-hub-signature-256", "sha256=invalid")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_github_webhook_missing_signature() {
    let app = create_test_app();

    let body = r#"{"action": "opened", "repository": {"full_name": "test/repo", "clone_url": "https://github.com/test/repo.git"}}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_gitlab_webhook_valid_token() {
    let app = create_test_app();

    let body = r#"{
        "object_kind": "merge_request",
        "project": {
            "id": 123,
            "path_with_namespace": "test/repo",
            "git_http_url": "https://gitlab.com/test/repo.git"
        },
        "object_attributes": {
            "iid": 1,
            "title": "Test MR",
            "description": "Test description",
            "source_branch": "feature-branch",
            "target_branch": "main",
            "last_commit": {
                "id": "abc123"
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
                .header("x-gitlab-token", "gitlab-secret")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_gitlab_webhook_invalid_token() {
    let app = create_test_app();

    let body = r#"{"object_kind": "merge_request", "project": {"id": 123, "path_with_namespace": "test/repo", "git_http_url": "https://gitlab.com/test/repo.git"}, "object_attributes": {"iid": 1, "title": "Test MR", "source_branch": "feature", "target_branch": "main", "last_commit": {"id": "abc"}, "state": "opened"}}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/gitlab")
                .header("x-gitlab-token", "wrong-token")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_gitlab_webhook_missing_token() {
    let app = create_test_app();

    let body = r#"{"object_kind": "merge_request", "project": {"id": 123, "path_with_namespace": "test/repo", "git_http_url": "https://gitlab.com/test/repo.git"}, "object_attributes": {"iid": 1, "title": "Test MR", "source_branch": "feature", "target_branch": "main", "last_commit": {"id": "abc"}, "state": "opened"}}"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/gitlab")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_gitlab_webhook_non_merge_request() {
    let app = create_test_app();

    let body = r#"{
        "object_kind": "push",
        "project": {
            "id": 123,
            "path_with_namespace": "test/repo",
            "git_http_url": "https://gitlab.com/test/repo.git"
        },
        "object_attributes": {
            "iid": 1,
            "title": "Test",
            "description": null,
            "source_branch": "feature",
            "target_branch": "main",
            "last_commit": {"id": "abc"},
            "state": "opened",
            "action": null
        }
    }"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/gitlab")
                .header("x-gitlab-token", "gitlab-secret")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

fn compute_github_signature(secret: &str, body: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    format!("sha256={}", hex::encode(code_bytes))
}
