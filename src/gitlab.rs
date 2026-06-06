use axum::{
    extract::State,
    http::{StatusCode, HeaderMap},
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
    pub object_attributes: Option<MergeRequest>,
    pub merge_request: Option<MergeRequest>,
    pub build_status: Option<String>,
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
    body: axum::body::Bytes,
) -> StatusCode {
    let payload: GitLabWebhook = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to parse GitLab webhook JSON: {}", e);
            return StatusCode::BAD_REQUEST;
        }
    };
    let gitlab_config = match state.config.gitlab.as_ref() {
        Some(cfg) => cfg,
        None => {
            warn!("GitLab webhook received but no GitLab config found");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    if !verify_gitlab_token(&headers, &gitlab_config.webhook_secret) {
        warn!("GitLab webhook token verification failed");
        state.metrics.record_webhook_rejected();
        return StatusCode::UNAUTHORIZED;
    }

    info!(
        "Received GitLab webhook: {} for {}",
        payload.object_kind, payload.project.path_with_namespace
    );

    let mr = match payload.object_kind.as_str() {
        "merge_request" => payload.object_attributes,
        "pipeline" => payload.merge_request,
        _ => None,
    };

    let mr = match mr {
        Some(mr) => mr,
        None => return StatusCode::OK,
    };

    let mr_title = mr.title.clone();

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
    let base_url = gitlab_config.base_url.clone();
    let ci_cd_enabled = gitlab_config.ci_cd_enabled;
    let batching_config = state.config.batching_config().clone();
    let use_batching = batching_config.enabled;
    let batch_timeout = Duration::from_secs(batching_config.timeout_seconds);
    let database = state.database.clone();

    let review_start = std::time::Instant::now();
    let metrics = state.metrics.clone();

    tokio::spawn(async move {
        info!("Processing GitLab MR !{}: {}", mr_iid, mr_title);

        let engine = ReviewEngine::from_diff_filter_config(state.config.diff_filter_config());

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
                        &base_url,
                        Some(&database),
                        ci_cd_enabled,
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
                info!("Empty diff for MR !{}, skipping review", mr_iid);
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

            let verdict = format!("{:?}", review.verdict);
            let repo_name = format!("gitlab/{}", project_id);

            if let Err(e) = post_review(
                project_id,
                mr_iid,
                &head_sha,
                &review,
                &access_token,
                &base_url,
                Some(&database),
                ci_cd_enabled,
            ).await {
                error!("Failed to post GitLab review: {}", e);
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
    project_id: u64,
    mr_iid: u64,
    head_sha: &str,
    batch: &crate::review::ReviewBatch,
    access_token: &str,
    base_url: &str,
    database: Option<&crate::db::Database>,
    ci_cd_enabled: bool,
    metrics: &crate::metrics::Metrics,
    review_start: std::time::Instant,
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
    let repo_name = format!("gitlab/{}", project_id);

    if let Err(e) = post_review(project_id, mr_iid, head_sha, &review, access_token, base_url, database, ci_cd_enabled).await {
        error!("Failed to post GitLab batch review: {}", e);
        metrics.record_review_failed();
    } else {
        metrics.record_review(&verdict, &repo_name, Some(review_start));
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn post_review(
    project_id: u64,
    mr_iid: u64,
    head_sha: &str,
    review: &crate::inline_comments::StructuredReview,
    access_token: &str,
    base_url: &str,
    database: Option<&crate::db::Database>,
    ci_cd_enabled: bool,
) -> anyhow::Result<()> {
    if !review.inline_comments.is_empty() {
        for comment in &review.inline_comments {
            post_inline_discussion(
                project_id,
                mr_iid,
                head_sha,
                comment,
                access_token,
                base_url,
            ).await?;
        }
    }

    let body = if ci_cd_enabled {
        format_ci_summary_body(review)
    } else {
        format_summary_body(review)
    };
    post_review_note(project_id, mr_iid, &body, access_token, base_url).await?;

    if let Some(db) = database {
        let verdict = format!("{:?}", review.verdict);
        let repo = format!("gitlab/{}", project_id);
        let _ = db.save_review(
            &repo,
            mr_iid as i64,
            "gitlab",
            head_sha,
            &verdict,
            &review.summary,
            review.inline_comments.len() as i64,
        ).await;
    }

    Ok(())
}

async fn post_inline_discussion(
    project_id: u64,
    mr_iid: u64,
    head_sha: &str,
    comment: &crate::inline_comments::InlineComment,
    access_token: &str,
    base_url: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v4/projects/{}/merge_requests/{}/discussions",
        base_url, project_id, mr_iid
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
    base_url: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v4/projects/{}/merge_requests/{}/notes",
        base_url, project_id, mr_iid
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

fn format_ci_summary_body(review: &crate::inline_comments::StructuredReview) -> String {
    let verdict_emoji = match review.verdict {
        ReviewVerdict::Approve => "✅",
        ReviewVerdict::RequestChanges => "❌",
        ReviewVerdict::Comment => "💬",
    };

    let mut body = format!(
        "🦈 **SentryShark CI/CD Review**\n\n{} **Verdict:** {:?}\n\n{}",
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

    body.push_str("\n\n*This review was triggered by a CI/CD pipeline event.*");
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

    #[test]
    fn test_format_ci_summary_body() {
        let review = crate::inline_comments::StructuredReview {
            verdict: ReviewVerdict::Comment,
            summary: "Some suggestions".to_string(),
            inline_comments: vec![],
        };

        let body = format_ci_summary_body(&review);
        assert!(body.contains("💬"));
        assert!(body.contains("CI/CD Review"));
        assert!(body.contains("triggered by a CI/CD pipeline event"));
    }
}
