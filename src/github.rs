use axum::{
    extract::State,
    body::Bytes,
    http::{StatusCode, HeaderMap},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{AppState, review::{ReviewEngine, CommitRange}, llm::LlmClient};
use crate::config::GitHubConfig;
use crate::inline_comments::ReviewVerdict;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize, Deserialize)]
struct GitHubJwtClaims {
    iss: String,
    iat: usize,
    exp: usize,
}

#[derive(Debug, Deserialize)]
struct GitHubInstallationToken {
    token: String,
    #[allow(dead_code)]
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct GitHubInstallation {
    id: u64,
}

#[derive(Clone)]
pub struct GitHubAuth {
    config: GitHubConfig,
    cached_token: Arc<Mutex<Option<(String, SystemTime)>>>,
}

impl GitHubAuth {
    pub fn new(config: GitHubConfig) -> Self {
        Self {
            config,
            cached_token: Arc::new(Mutex::new(None)),
        }
    }

    fn generate_jwt(&self) -> anyhow::Result<String> {
        let private_key_pem = std::fs::read_to_string(&self.config.private_key_path)?;
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key_pem.as_bytes())?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs() as usize;
        let exp = now + 600;

        let claims = GitHubJwtClaims {
            iss: self.config.app_id.clone(),
            iat: now,
            exp,
        };

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let jwt = jsonwebtoken::encode(&header, &claims, &encoding_key)?;

        Ok(jwt)
    }

    async fn get_installation_token(&self) -> anyhow::Result<String> {
        let mut cached = self.cached_token.lock().await;
        if let Some((token, expires)) = cached.as_ref() {
            let now = SystemTime::now();
            if now < *expires {
                info!("Using cached GitHub installation token");
                return Ok(token.clone());
            }
        }

        let jwt = self.generate_jwt()?;

        let installation_id = if let Some(id) = self.config.installation_id {
            id
        } else {
            let client = reqwest::Client::new();
            let response = client
                .get("https://api.github.com/app/installations")
                .header("Authorization", format!("Bearer {}", jwt))
                .header("Accept", "application/vnd.github.v3+json")
                .header("User-Agent", "SentryShark")
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("GitHub API error {}: {}", status, text));
            }

            let installations: Vec<GitHubInstallation> = response.json().await?;
            installations
                .into_iter()
                .next()
                .map(|i| i.id)
                .ok_or_else(|| anyhow::anyhow!("No GitHub App installations found"))?
        };

        let client = reqwest::Client::new();
        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            installation_id
        );

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "SentryShark")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("GitHub API error {}: {}", status, text));
        }

        let token_response: GitHubInstallationToken = response.json().await?;
        let expires = SystemTime::now() + Duration::from_secs(3300);

        info!("Obtained new GitHub installation token");
        *cached = Some((token_response.token.clone(), expires));

        Ok(token_response.token)
    }

    async fn get_token(&self) -> anyhow::Result<String> {
        if self.config.use_app_auth {
            self.get_installation_token().await
        } else {
            std::fs::read_to_string(&self.config.private_key_path)
                .or_else(|_| Ok(self.config.app_id.clone()))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GitHubWebhook {
    pub action: String,
    pub pull_request: Option<PullRequest>,
    pub repository: Repository,
}

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub head: BranchRef,
    pub base: BranchRef,
}

#[derive(Debug, Deserialize)]
pub struct BranchRef {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub sha: String,
    pub repo: Option<RepoRef>,
}

#[derive(Debug, Deserialize)]
pub struct RepoRef {
    pub clone_url: String,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub clone_url: String,
}

#[derive(Debug, Serialize)]
pub struct GitHubReviewComment {
    pub body: String,
    pub path: String,
    pub line: u32,
    pub side: String,
}

#[derive(Debug, Serialize)]
pub struct GitHubPullRequestReview {
    pub body: String,
    pub event: String,
    pub comments: Vec<GitHubReviewComment>,
}

#[derive(Debug, Serialize)]
pub struct GitHubIssueComment {
    pub body: String,
}

fn verify_github_signature(secret: &str, body: &Bytes, signature: &str) -> bool {
    let sig = signature.strip_prefix("sha256=").unwrap_or(signature);
    let sig_bytes = match hex::decode(sig) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(body);
    let result = mac.finalize();
    let code_bytes = result.into_bytes();

    code_bytes.as_slice() == sig_bytes.as_slice()
}

pub async fn webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let github_config = match state.config.github.as_ref() {
        Some(cfg) => cfg.clone(),
        None => {
            warn!("GitHub webhook received but no GitHub config found");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    if !verify_github_signature(&github_config.webhook_secret, &body, signature) {
        warn!("GitHub webhook signature verification failed");
        state.metrics.record_webhook_rejected();
        return StatusCode::UNAUTHORIZED;
    }

    let payload: GitHubWebhook = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to parse GitHub webhook JSON: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };

    info!("Received GitHub webhook: {} for {}", payload.action, payload.repository.full_name);

    if payload.action != "opened" && payload.action != "synchronize" {
        return StatusCode::OK;
    }

    let pr = match payload.pull_request {
        Some(pr) => pr,
        None => {
            warn!("No PR data in webhook");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Use the head repo's clone URL if available (for cross-repo PRs), otherwise default
    let repo_url = pr.head.repo.as_ref()
        .map(|r| r.clone_url.clone())
        .unwrap_or_else(|| payload.repository.clone_url.clone());

    let base = pr.base.ref_.clone();
    let head = pr.head.ref_.clone();
    let head_sha = pr.head.sha.clone();
    let pr_number = pr.number;
    let pr_title = pr.title.clone();
    let repo_name = payload.repository.full_name.clone();

    let review_config = state.config.review_config().clone();
    let llm = LlmClient::new(
        state.config.llm.base_url.clone(),
        state.config.llm.model.clone(),
        state.config.llm.max_tokens,
        state.config.llm.temperature,
        review_config,
    );

    let github_app_id = github_config.app_id.clone();
    let github_key_path = github_config.private_key_path.clone();
    let batching_config = state.config.batching_config().clone();
    let use_batching = batching_config.enabled;
    let batch_timeout = Duration::from_secs(batching_config.timeout_seconds);

    tokio::spawn(async move {
        info!("Processing GitHub PR #{}: {}", pr_number, pr_title);

        let engine = ReviewEngine::from_diff_filter_config(state.config.diff_filter_config());

        let review_start = std::time::Instant::now();
        let metrics = state.metrics.clone();

        if use_batching {
            let batch_key = format!("{}:{}", repo_name, pr_number);
            let commit = CommitRange { base: base.clone(), head: head.clone() };
            
            let new_batch = engine.add_to_batch(
                &batch_key,
                &repo_url,
                commit,
            ).await;

            if new_batch.is_some() {
                // First commit in batch, wait for timeout or more commits
                tokio::time::sleep(batch_timeout).await;
                
                if let Some(batch) = engine.get_batch(&batch_key).await {
                    let db = state.database.clone();
                    process_batch_review(
                        &engine,
                        &llm,
                        &repo_name,
                        pr_number,
                        &head_sha,
                        &batch,
                        &github_config,
                        Some(&db),
                        &metrics,
                        review_start,
                    ).await;
                }
            }
        } else {
            let diff = match engine.clone_and_diff(&repo_url, &base, &head) {
                Ok(d) => d,
                Err(e) => {
                    error!("Failed to clone and diff: {}", e);
                    metrics.record_review_failed();
                    return;
                }
            };

            if diff.is_empty() {
                info!("Empty diff for PR #{}, skipping review", pr_number);
                return;
            }

            let review = match llm.review_code(&diff).await {
                Ok(r) => r,
                Err(e) => {
                    error!("LLM review failed: {}", e);
                    metrics.record_review_failed();
                    return;
                }
            };

            let config = GitHubConfig {
                webhook_secret: String::new(),
                app_id: github_app_id,
                private_key_path: github_key_path,
                use_app_auth: github_config.use_app_auth,
                installation_id: github_config.installation_id,
            };

            let db = state.database.clone();
            let verdict = format!("{:?}", review.verdict);

            if let Err(e) = post_review(
                &repo_name,
                pr_number,
                &head_sha,
                &review,
                &config,
                Some(&db),
            ).await {
                error!("Failed to post GitHub review: {}", e);
                metrics.record_review_failed();
            } else {
                metrics.record_review(&verdict, &repo_name, Some(review_start));
            }
        }
    });

    StatusCode::OK
}

#[allow(clippy::too_many_arguments)]
async fn process_batch_review(
    engine: &ReviewEngine,
    llm: &LlmClient,
    repo_name: &str,
    pr_number: u64,
    head_sha: &str,
    batch: &crate::review::ReviewBatch,
    config: &GitHubConfig,
    database: Option<&crate::db::Database>,
    metrics: &crate::metrics::Metrics,
    review_start: std::time::Instant,
) {
    info!(
        "Processing batch review for {}/{} with {} commits",
        repo_name, pr_number, batch.commits.len()
    );

    let diff = match engine.clone_and_diff_batch(
        &batch.repo_url,
        &batch.commits,
    ) {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to generate batch diff: {}", e);
            metrics.record_review_failed();
            return;
        }
    };

    if diff.is_empty() {
        info!("Empty diff for batch review, skipping");
        return;
    }

    let review = match llm.review_code(&diff).await {
        Ok(r) => r,
        Err(e) => {
            error!("LLM batch review failed: {}", e);
            metrics.record_review_failed();
            return;
        }
    };

    let verdict = format!("{:?}", review.verdict);

    if let Err(e) = post_review(repo_name, pr_number, head_sha, &review, config, database).await {
        error!("Failed to post GitHub batch review: {}", e);
        metrics.record_review_failed();
    } else {
        metrics.record_review(&verdict, repo_name, Some(review_start));
    }
}

pub async fn post_review(
    repo: &str,
    pr_number: u64,
    head_sha: &str,
    review: &crate::inline_comments::StructuredReview,
    config: &GitHubConfig,
    database: Option<&crate::db::Database>,
) -> anyhow::Result<()> {
    let auth = GitHubAuth::new(config.clone());
    let token = auth.get_token().await?;

    if !review.inline_comments.is_empty() {
        post_pull_request_review(repo, pr_number, head_sha, review, &token).await?;
    } else {
        let body = format_summary_body(review);
        post_issue_comment(repo, pr_number, &body, &token).await?;
    }

    if let Some(db) = database {
        let verdict = format!("{:?}", review.verdict);
        let _ = db.save_review(
            repo,
            pr_number as i64,
            "github",
            head_sha,
            &verdict,
            &review.summary,
            review.inline_comments.len() as i64,
        ).await;
    }

    Ok(())
}

async fn post_pull_request_review(
    repo: &str,
    pr_number: u64,
    _head_sha: &str,
    review: &crate::inline_comments::StructuredReview,
    token: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/pulls/{}/reviews",
        repo, pr_number
    );

    let event = match review.verdict {
        ReviewVerdict::Approve => "APPROVE",
        ReviewVerdict::RequestChanges => "REQUEST_CHANGES",
        ReviewVerdict::Comment => "COMMENT",
    };

    let comments: Vec<GitHubReviewComment> = review.inline_comments.iter().map(|c| {
        GitHubReviewComment {
            body: c.body.clone(),
            path: c.file_path.clone(),
            line: c.line,
            side: "RIGHT".to_string(),
        }
    }).collect();

    let review_request = GitHubPullRequestReview {
        body: format!("🦈 **SentryShark Code Review**\n\n{}", review.summary),
        event: event.to_string(),
        comments,
    };

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "SentryShark")
        .json(&review_request)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("GitHub PR review API error {}: {}", status, text));
    }

    info!("Posted PR review to {}/pulls/{} with verdict {:?}", repo, pr_number, review.verdict);
    Ok(())
}

async fn post_issue_comment(
    repo: &str,
    pr_number: u64,
    body: &str,
    token: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/issues/{}/comments",
        repo, pr_number
    );

    let comment = GitHubIssueComment {
        body: body.to_string(),
    };

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "SentryShark")
        .json(&comment)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("GitHub API error {}: {}", status, text));
    }

    info!("Posted issue comment to {}/issues/{}", repo, pr_number);
    Ok(())
}

fn format_summary_body(review: &crate::inline_comments::StructuredReview) -> String {
    let verdict_emoji = match review.verdict {
        ReviewVerdict::Approve => "✅",
        ReviewVerdict::RequestChanges => "❌",
        ReviewVerdict::Comment => "💬",
    };

    let mut body = format!(
        "🦈 **SentryShark Code Review**\n\n{} **Verdict:** {:?}\n\n{}",
        verdict_emoji,
        review.verdict,
        review.summary
    );

    if !review.inline_comments.is_empty() {
        body.push_str("\n\n**Inline Comments:**\n");
        for comment in &review.inline_comments {
            body.push_str(&format!(
                "- `{}:{}` - {}\n",
                comment.file_path,
                comment.line,
                comment.body.lines().next().unwrap_or("")
            ));
        }
    }

    body
}

pub async fn post_review_comment(
    repo: &str,
    pr_number: u64,
    body: &str,
    config: &GitHubConfig,
) -> anyhow::Result<()> {
    let auth = GitHubAuth::new(config.clone());
    let token = auth.get_token().await?;
    post_issue_comment(repo, pr_number, body, &token).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inline_comments::{InlineComment, ReviewVerdict, StructuredReview};

    #[test]
    fn test_verify_github_signature() {
        let secret = "mysecret";
        let body = Bytes::from_static(b"test payload");
        
        // Compute correct signature
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&body);
        let result = mac.finalize();
        let code_bytes = result.into_bytes();
        let sig = format!("sha256={}", hex::encode(code_bytes));

        assert!(verify_github_signature(secret, &body, &sig));
        assert!(!verify_github_signature(secret, &body, "sha256=invalid"));
        assert!(!verify_github_signature("wrongsecret", &body, &sig));
    }

    #[test]
    fn test_format_summary_body() {
        let review = StructuredReview {
            verdict: ReviewVerdict::Approve,
            summary: "Looks good!".to_string(),
            inline_comments: vec![
                InlineComment {
                    file_path: "src/main.rs".to_string(),
                    line: 42,
                    body: "Consider error handling".to_string(),
                }
            ],
        };

        let body = format_summary_body(&review);
        assert!(body.contains("✅"));
        assert!(body.contains("Approve"));
        assert!(body.contains("src/main.rs:42"));
    }

    #[test]
    fn test_format_summary_body_request_changes() {
        let review = StructuredReview {
            verdict: ReviewVerdict::RequestChanges,
            summary: "Needs work".to_string(),
            inline_comments: vec![],
        };

        let body = format_summary_body(&review);
        assert!(body.contains("❌"));
        assert!(body.contains("RequestChanges"));
    }

    #[test]
    fn test_github_auth_token_fallback() {
        let config = GitHubConfig {
            webhook_secret: "secret".to_string(),
            app_id: "test-app-id".to_string(),
            private_key_path: "/tmp/nonexistent.pem".to_string(),
            use_app_auth: false,
            installation_id: None,
        };

        let auth = GitHubAuth::new(config);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let token = rt.block_on(auth.get_token()).unwrap();
        assert_eq!(token, "test-app-id");
    }
}
