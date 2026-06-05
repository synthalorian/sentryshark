use axum::{
    extract::State,
    body::Bytes,
    http::{StatusCode, HeaderMap},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::{AppState, review::ReviewEngine, llm::LlmClient};

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
    pub commit_id: String,
    pub path: String,
    pub line: u32,
    pub side: String,
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
        Some(cfg) => cfg,
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
    let pr_number = pr.number;
    let pr_title = pr.title.clone();
    let repo_name = payload.repository.full_name.clone();

    let llm = LlmClient::new(
        state.config.llm.base_url.clone(),
        state.config.llm.model.clone(),
        state.config.llm.max_tokens,
        state.config.llm.temperature,
    );

    let github_app_id = github_config.app_id.clone();
    let github_key_path = github_config.private_key_path.clone();

    tokio::spawn(async move {
        info!("Processing GitHub PR #{}: {}", pr_number, pr_title);

        let diff = match ReviewEngine::clone_and_diff(&repo_url, &base, &head) {
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

        let config = crate::config::GitHubConfig {
            webhook_secret: String::new(),
            app_id: github_app_id,
            private_key_path: github_key_path,
        };

        if let Err(e) = post_review_comment(
            &repo_name,
            pr_number,
            &review,
            &config,
        ).await {
            error!("Failed to post GitHub review comment: {}", e);
        }
    });

    StatusCode::OK
}

pub async fn post_review_comment(
    repo: &str,
    pr_number: u64,
    body: &str,
    config: &crate::config::GitHubConfig,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/issues/{}/comments",
        repo, pr_number
    );

    let comment = GitHubIssueComment {
        body: format!("🦈 **SentryShark Code Review**\n\n{}", body),
    };

    // Use a personal access token or app token. For v0.1.0, we use app_id as a placeholder for PAT.
    // In production, you should implement JWT-based GitHub App authentication.
    let token = std::fs::read_to_string(&config.private_key_path)
        .unwrap_or_else(|_| config.app_id.clone());

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

    info!("Posted review comment to {}/pulls/{}", repo, pr_number);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
