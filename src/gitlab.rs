use axum::{
    extract::State,
    http::{StatusCode, HeaderMap},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};
use std::time::Duration;

use crate::{AppState, review::{ReviewEngine, CommitRange}, llm::LlmClient};
use crate::inline_comments::{ReviewVerdict};

#[derive(Debug, Deserialize)]
pub struct GitLabWebhook {
    pub object_kind: String,
    pub project: Project,
    pub object_attributes: MergeRequest,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub id: u64,
    pub path_with_namespace: String,
    pub git_http_url: String,
}

#[derive(Debug, Deserialize)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub last_commit: Commit,
    pub state: String,
    pub action: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Commit {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct GitLabNote {
    pub body: String,
}

#[derive(Debug, Serialize)]
pub struct GitLabDiscussion {
    pub body: String,
    pub position: GitLabPosition,
}

#[derive(Debug, Serialize)]
pub struct GitLabPosition {
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: String,
    pub position_type: String,
    pub new_path: String,
    pub new_line: u32,
}

fn verify_gitlab_token(headers: &HeaderMap, expected: &str) -> bool {
    let token = headers
        .get("x-gitlab-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    if token.len() != expected.len() {
        return false;
    }
    token.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub async fn webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitLabWebhook>,
) -> StatusCode {
    let gitlab_config = match state.config.gitlab.as_ref() {
        Some(cfg) => cfg,
        None => {
            warn!("GitLab webhook received but no GitLab config found");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    if !verify_gitlab_token(&headers, &gitlab_config.webhook_secret) {
        warn!("GitLab webhook token verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    info!(
        "Received GitLab webhook: {} for {}",
        payload.object_kind, payload.project.path_with_namespace
    );

    if payload.object_kind != "merge_request" {
        return StatusCode::OK;
    }

    let mr = payload.object_attributes;
    
    let mr_title = mr.title.clone();
    
    // Only review on open or update actions
    let action = mr.action.as_deref().unwrap_or("");
    if mr.state != "opened" && action != "open" && action != "update" && action != "merge" {
        return StatusCode::OK;
    }

    let repo_url = payload.project.git_http_url;
    let base = mr.target_branch;
    let head = mr.source_branch;
    let head_sha = mr.last_commit.id.clone();
    let mr_iid = mr.iid;
    let project_id = payload.project.id;
    let project_path = payload.project.path_with_namespace;

    let review_config = state.config.review_config().clone();
    let llm = LlmClient::new(
        state.config.llm.base_url.clone(),
        state.config.llm.model.clone(),
        state.config.llm.max_tokens,
        state.config.llm.temperature,
        review_config,
    );

    let access_token = gitlab_config.access_token.clone();
    let batching_config = state.config.batching_config().clone();
    let use_batching = batching_config.enabled;
    let batch_timeout = Duration::from_secs(batching_config.timeout_seconds);

    tokio::spawn(async move {
        info!("Processing GitLab MR !{}: {}", mr_iid, mr_title);

        let engine = ReviewEngine::new(
            &crate::config::AppConfig {
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
            }
        );

        if use_batching {
            let batch_key = format!("{}:{}", project_path, mr_iid);
            let commit = CommitRange { base: base.clone(), head: head.clone() };
            
            let new_batch = engine.add_to_batch(
                &batch_key,
                &repo_url,
                commit,
            ).await;

            if new_batch.is_some() {
                tokio::time::sleep(batch_timeout).await;
                
                if let Some(batch) = engine.get_batch(&batch_key).await {
                    process_batch_review(
                        &engine,
                        &llm,
                        project_id,
                        mr_iid,
                        &head_sha,
                        &batch,
                        &access_token,
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
                info!("Empty diff for MR !{}, skipping review", mr_iid);
                return;
            }

            let review = match llm.review_code(&diff).await {
                Ok(r) => r,
                Err(e) => {
                    error!("LLM review failed: {}", e);
                    return;
                }
            };

            if let Err(e) = post_review(
                project_id,
                mr_iid,
                &head_sha,
                &review,
                &access_token,
            ).await {
                error!("Failed to post GitLab review: {}", e);
            }
        }
    });

    StatusCode::OK
}

async fn process_batch_review(
    engine: &ReviewEngine,
    llm: &LlmClient,
    project_id: u64,
    mr_iid: u64,
    head_sha: &str,
    batch: &crate::review::ReviewBatch,
    access_token: &str,
) {
    info!(
        "Processing batch review for !{} in project {} with {} commits",
        mr_iid, project_id, batch.commits.len()
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

    if let Err(e) = post_review(project_id, mr_iid, head_sha, &review, access_token).await {
        error!("Failed to post GitLab batch review: {}", e);
    }
}

pub async fn post_review(
    project_id: u64,
    mr_iid: u64,
    head_sha: &str,
    review: &crate::inline_comments::StructuredReview,
    access_token: &str,
) -> anyhow::Result<()> {
    // Post inline comments as discussions if available
    if !review.inline_comments.is_empty() {
        for comment in &review.inline_comments {
            post_inline_discussion(
                project_id,
                mr_iid,
                head_sha,
                comment,
                access_token,
            ).await?;
        }
    }

    // Always post summary note
    let body = format_summary_body(review);
    post_review_note(project_id, mr_iid, &body, access_token).await?;

    Ok(())
}

async fn post_inline_discussion(
    project_id: u64,
    mr_iid: u64,
    head_sha: &str,
    comment: &crate::inline_comments::InlineComment,
    access_token: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://gitlab.com/api/v4/projects/{}/merge_requests/{}/discussions",
        project_id, mr_iid
    );

    let discussion = GitLabDiscussion {
        body: comment.body.clone(),
        position: GitLabPosition {
            base_sha: head_sha.to_string(),
            head_sha: head_sha.to_string(),
            start_sha: head_sha.to_string(),
            position_type: "text".to_string(),
            new_path: comment.file_path.clone(),
            new_line: comment.line,
        },
    };

    let response = client
        .post(&url)
        .header("PRIVATE-TOKEN", access_token)
        .header("User-Agent", "SentryShark")
        .json(&discussion)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("GitLab discussion API error {}: {}", status, text));
    }

    info!(
        "Posted inline discussion to !{} on {}:{}",
        mr_iid, comment.file_path, comment.line
    );
    Ok(())
}

pub async fn post_review_note(
    project_id: u64,
    mr_iid: u64,
    body: &str,
    access_token: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://gitlab.com/api/v4/projects/{}/merge_requests/{}/notes",
        project_id, mr_iid
    );

    let note = GitLabNote {
        body: body.to_string(),
    };

    let response = client
        .post(&url)
        .header("PRIVATE-TOKEN", access_token)
        .header("User-Agent", "SentryShark")
        .json(&note)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("GitLab API error {}: {}", status, text));
    }

    info!("Posted review note to !{} in project {}", mr_iid, project_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn test_verify_gitlab_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-gitlab-token", "mysecret".parse().unwrap());

        assert!(verify_gitlab_token(&headers, "mysecret"));
        assert!(!verify_gitlab_token(&headers, "wrongsecret"));
        assert!(!verify_gitlab_token(&headers, ""));
        assert!(!verify_gitlab_token(&HeaderMap::new(), "mysecret"));
    }

    #[test]
    fn test_format_summary_body() {
        let review = crate::inline_comments::StructuredReview {
            verdict: ReviewVerdict::Approve,
            summary: "Looks good!".to_string(),
            inline_comments: vec![
                crate::inline_comments::InlineComment {
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
}
