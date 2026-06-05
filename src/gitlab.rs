use axum::{
    extract::State,
    http::{StatusCode, HeaderMap},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

use crate::{AppState, review::ReviewEngine, llm::LlmClient};

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
    let mr_iid = mr.iid;
    let project_id = payload.project.id;
    let _project_path = payload.project.path_with_namespace;

    let llm = LlmClient::new(
        state.config.llm.base_url.clone(),
        state.config.llm.model.clone(),
        state.config.llm.max_tokens,
        state.config.llm.temperature,
    );

    let access_token = gitlab_config.access_token.clone();

    tokio::spawn(async move {
        info!("Processing GitLab MR !{}: {}", mr_iid, mr_title);

        let diff = match ReviewEngine::clone_and_diff(&repo_url, &base, &head) {
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

        if let Err(e) = post_review_note(
            project_id,
            mr_iid,
            &review,
            &access_token,
        ).await {
            error!("Failed to post GitLab review note: {}", e);
        }
    });

    StatusCode::OK
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
        body: format!("🦈 **SentryShark Code Review**\n\n{}", body),
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
}
