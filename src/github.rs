use axum::{
    extract::State,
    body::Bytes,
    http::{StatusCode, HeaderMap},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::Duration;

use crate::{AppState, review::{ReviewEngine, CommitRange}, llm::LlmClient};
use crate::config::GitHubConfig;
use crate::inline_comments::ReviewVerdict;

type HmacSha256 = Hmac<Sha256>;

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

        let engine = ReviewEngine::new(&crate::config::AppConfig {
            server: crate::config::ServerConfig { host: "0.0.0.0".to_string(), port: 3000 },
            github: None,
            gitlab: None,
            llm: crate::config::LlmConfig {
                provider: "llamacpp".to_string(),
                base_url: "http://localhost:8080".to_string(),
                model: "default".to_string(),
                max_tokens: 4096,
                temperature: 0.1,
            },
            review: Some(crate::config::ReviewConfig::default()),
            diff_filter: Some(crate::config::DiffFilterConfig::default()),
            batching: Some(crate::config::BatchingConfig::default()),
        });

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
                    process_batch_review(
                        &engine,
                        &llm,
                        &repo_name,
                        pr_number,
                        &head_sha,
                        &batch,
                        &github_config,
                    ).await;
                }
            }
        } else {
            let diff = match engine.clone_and_diff(&repo_url, &base, &head) {
                Ok(d) => d,
                Err(e) => {
                    error!("Failed to clone and diff: {}", e);
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
                    return;
                }
            };

            let config = GitHubConfig {
                webhook_secret: String::new(),
                app_id: github_app_id,
                private_key_path: github_key_path,
            };

            if let Err(e) = post_review(
                &repo_name,
                pr_number,
                &head_sha,
                &review,
                &config,
            ).await {
                error!("Failed to post GitHub review: {}", e);
            }
        }
    });

    StatusCode::OK
}

async fn process_batch_review(
    engine: &ReviewEngine,
    llm: &LlmClient,
    repo_name: &str,
    pr_number: u64,
    head_sha: &str,
    batch: &crate::review::ReviewBatch,
    config: &GitHubConfig,
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
            return;
        }
    };

    if let Err(e) = post_review(repo_name, pr_number, head_sha, &review, config).await {
        error!("Failed to post GitHub batch review: {}", e);
    }
}

pub async fn post_review(
    repo: &str,
    pr_number: u64,
    head_sha: &str,
    review: &crate::inline_comments::StructuredReview,
    config: &GitHubConfig,
) -> anyhow::Result<()> {
    let token = std::fs::read_to_string(&config.private_key_path)
        .unwrap_or_else(|_| config.app_id.clone());

    // Post inline comments via PR review API if available
    if !review.inline_comments.is_empty() {
        post_pull_request_review(repo, pr_number, head_sha, review, &token).await?;
    } else {
        // Fall back to issue comment for summary only
        let body = format_summary_body(review);
        post_issue_comment(repo, pr_number, &body, &token).await?;
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
        .header("Authorization", format!("token {}", token))
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
        .header("Authorization", format!("token {}", token))
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
    let token = std::fs::read_to_string(&config.private_key_path)
        .unwrap_or_else(|_| config.app_id.clone());
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
}
