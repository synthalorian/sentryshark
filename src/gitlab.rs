use axum::{
    extract::State,
    http::{StatusCode, HeaderMap},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error, instrument};
use std::time::Duration;

use crate::{AppState, review::{ReviewEngine, CommitRange}, llm::LlmClient};
use crate::inline_comments::{ReviewVerdict, SeverityLevel, ReviewParser};
use crate::auto_approve::{AutoApprover, auto_approve_message};
use crate::cache::ReviewCache;

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

    let auto_approve_config = crate::auto_approve::AutoApproveConfig {
        enabled: state.config.auto_approve_config().enabled,
        docs_patterns: state.config.auto_approve_config().docs_patterns.clone(),
        skip_lockfiles: state.config.auto_approve_config().skip_lockfiles,
        skip_whitespace: state.config.auto_approve_config().skip_whitespace,
    };

    let cache = if state.config.cache_config().enabled {
        Some(ReviewCache::new(
            state.database.clone(),
            state.config.cache_config().ttl_hours,
        ))
    } else {
        None
    };

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

            // Check for trivial changes and auto-approve
            if let Some(reason) = AutoApprover::is_trivial(&diff, &auto_approve_config) {
                info!("Auto-approving MR !{}: {}", mr_iid, reason);
                let body = auto_approve_message(&reason);
                if let Err(e) = post_review_note(project_id, mr_iid, &body, &access_token, &base_url).await {
                    error!("Failed to post auto-approve note: {}", e);
                }
                metrics.record_auto_approve();
                return;
            }

            // Check cache before LLM call
            if let Some(ref cache) = cache {
                match cache.get(&diff).await {
                    Ok(Some(cached_review)) => {
                        info!("Using cached review for MR !{}", mr_iid);
                        metrics.record_cache_hit();
                let repo_name = format!("gitlab/{}", project_id);
                        let verdict = format!("{:?}", cached_review.verdict);
                        if let Err(e) = post_review(
                            project_id,
                            mr_iid,
                            &head_sha,
                            &cached_review,
                            &access_token,
                            &base_url,
                            Some(&database),
                            ci_cd_enabled,
                        ).await {
                            error!("Failed to post cached review: {}", e);
                            metrics.record_review_failed();
                        } else {
                            metrics.record_review(&verdict, &repo_name, Some(review_start));
                        }
                        return;
                    }
                    Ok(None) => {
                        metrics.record_cache_miss();
                    }
                    Err(e) => {
                        warn!("Cache lookup failed: {}", e);
                    }
                }
            }

            let review = match llm.review_code(&diff).await {
                Ok(r) => r,
                Err(e) => {
                    error!("LLM review failed: {}", e);
                    metrics.record_review_failed();
                    return;
                }
            };

            // Cache the result
            if let Some(ref cache) = cache {
                if let Err(e) = cache.set(&diff, &review).await {
                    warn!("Failed to cache review: {}", e);
                }
            }

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
#[instrument(skip(review, access_token, database))]
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
        let critical_count = review.inline_comments.iter()
            .filter(|c| matches!(c.severity, SeverityLevel::Critical)).count() as i64;
        let warning_count = review.inline_comments.iter()
            .filter(|c| matches!(c.severity, SeverityLevel::Warning)).count() as i64;
        let info_count = review.inline_comments.iter()
            .filter(|c| matches!(c.severity, SeverityLevel::Info)).count() as i64;
        let _ = db.save_review(
            &repo,
            mr_iid as i64,
            "gitlab",
            head_sha,
            &verdict,
            &review.summary,
            review.inline_comments.len() as i64,
            critical_count,
            warning_count,
            info_count,
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

    let severity_label = ReviewParser::format_severity_label(&comment.severity);
    let discussion = GitLabDiscussion {
        body: format!("{} {}", severity_label, comment.body),
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

pub fn format_summary_body(review: &crate::inline_comments::StructuredReview) -> String {
    let verdict_emoji = match review.verdict {
        ReviewVerdict::Approve => "\u{2705}",
        ReviewVerdict::RequestChanges => "\u{274c}",
        ReviewVerdict::Comment => "\u{1f4ac}",
    };

    let mut body = format!(
        "\u{1f988} **SentryShark Code Review**\n\n{} **Verdict:** {:?}\n\n{}",
        verdict_emoji,
        review.verdict,
        review.summary
    );

    if !review.inline_comments.is_empty() {
        body.push_str("\n\n**Inline Comments:**\n");
        for comment in &review.inline_comments {
            let severity_label = ReviewParser::format_severity_label(&comment.severity);
            body.push_str(&format!(
                "- `{}:{}` - {} {}\n",
                comment.file_path,
                comment.line,
                severity_label,
                comment.body.lines().next().unwrap_or("")
            ));
        }
    }

    body
}

pub fn format_ci_summary_body(review: &crate::inline_comments::StructuredReview) -> String {
    let verdict_emoji = match review.verdict {
        ReviewVerdict::Approve => "\u{2705}",
        ReviewVerdict::RequestChanges => "\u{274c}",
        ReviewVerdict::Comment => "\u{1f4ac}",
    };

    let mut body = format!(
        "\u{1f988} **SentryShark CI/CD Review**\n\n{} **Verdict:** {:?}\n\n{}",
        verdict_emoji,
        review.verdict,
        review.summary
    );

    if !review.inline_comments.is_empty() {
        body.push_str("\n\n**Inline Comments:**\n");
        for comment in &review.inline_comments {
            let severity_label = ReviewParser::format_severity_label(&comment.severity);
            body.push_str(&format!(
                "- `{}:{}` - {} {}\n",
                comment.file_path,
                comment.line,
                severity_label,
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
                    severity: SeverityLevel::Warning,
                }
            ],
        };

        let body = format_summary_body(&review);
        assert!(body.contains("\u{2705}"));
        assert!(body.contains("Approve"));
        assert!(body.contains("src/main.rs:42"));
        assert!(body.contains("Warning"));
    }

    #[test]
    fn test_format_ci_summary_body() {
        let review = crate::inline_comments::StructuredReview {
            verdict: ReviewVerdict::Comment,
            summary: "Some suggestions".to_string(),
            inline_comments: vec![],
        };

        let body = format_ci_summary_body(&review);
        assert!(body.contains("\u{1f4ac}"));
        assert!(body.contains("CI/CD Review"));
        assert!(body.contains("triggered by a CI/CD pipeline event"));
    }
}
