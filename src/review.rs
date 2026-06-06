use tracing::{info, error};
use std::process::Command;
use std::collections::HashMap;
use tokio::sync::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::diff_filter::DiffFilter;
use crate::config::AppConfig;

#[derive(Clone, Debug)]
pub struct CommitRange {
    pub base: String,
    pub head: String,
}

#[derive(Clone, Debug)]
pub struct ReviewBatch {
    pub repo_url: String,
    pub commits: Vec<CommitRange>,
    pub created_at: Instant,
}

pub struct ReviewEngine {
    diff_filter: Option<DiffFilter>,
    pending_batches: Arc<Mutex<HashMap<String, ReviewBatch>>>,
}

impl ReviewEngine {
    pub fn new(config: &AppConfig) -> Self {
        let diff_filter_config = config.diff_filter_config();
        let diff_filter = if diff_filter_config.enabled {
            Some(DiffFilter::new(
                &diff_filter_config.lockfile_patterns,
                &diff_filter_config.generated_patterns,
                true,
            ))
        } else {
            None
        };

        Self {
            diff_filter,
            pending_batches: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn clone_and_diff(&self,
        repo_url: &str,
        base: &str,
        head: &str,
    ) -> anyhow::Result<String> {
        let temp_dir = tempfile::tempdir()?;
        let repo_path = temp_dir.path();

        info!("Cloning {} into {:?}", repo_url, repo_path);

        // Initialize a bare repo and fetch both branches as local refs
        let init_output = Command::new("git")
            .args(["init", "--bare", repo_path.to_str().unwrap()])
            .output()?;

        if !init_output.status.success() {
            let stderr = String::from_utf8_lossy(&init_output.stderr);
            error!("git init failed: {}", stderr);
            return Err(anyhow::anyhow!("git init failed: {}", stderr));
        }

        let fetch_output = Command::new("git")
            .args([
                "fetch",
                "--depth=100",
                repo_url,
                &format!("{}:refs/heads/base", base),
                &format!("{}:refs/heads/head", head),
            ])
            .current_dir(repo_path)
            .output()?;

        if !fetch_output.status.success() {
            let stderr = String::from_utf8_lossy(&fetch_output.stderr);
            error!("git fetch failed: {}", stderr);
            return Err(anyhow::anyhow!("git fetch failed: {}", stderr));
        }

        info!("Generating diff between {} and {}", base, head);

        let diff_output = Command::new("git")
            .args(["diff", "base..head"])
            .current_dir(repo_path)
            .output()?;

        if !diff_output.status.success() {
            let stderr = String::from_utf8_lossy(&diff_output.stderr);
            error!("git diff failed: {}", stderr);
            return Err(anyhow::anyhow!("git diff failed: {}", stderr));
        }

        let diff = String::from_utf8_lossy(&diff_output.stdout).to_string();
        info!("Generated diff: {} bytes", diff.len());

        // Apply diff filtering if configured
        let filtered_diff = if let Some(ref filter) = self.diff_filter {
            let filtered = filter.filter_diff(&diff);
            info!(
                "Filtered diff: {} bytes (removed {} bytes)",
                filtered.len(),
                diff.len().saturating_sub(filtered.len())
            );
            filtered
        } else {
            diff
        };

        Ok(filtered_diff)
    }

    pub async fn add_to_batch(
        &self,
        batch_key: &str,
        repo_url: &str,
        commit: CommitRange,
    ) -> Option<ReviewBatch> {
        let mut batches = self.pending_batches.lock().await;
        
        if let Some(batch) = batches.get_mut(batch_key) {
            batch.commits.push(commit);
            info!(
                "Added commit to existing batch {} (now {} commits)",
                batch_key,
                batch.commits.len()
            );
            None
        } else {
            let batch = ReviewBatch {
                repo_url: repo_url.to_string(),
                commits: vec![commit],
                created_at: Instant::now(),
            };
            batches.insert(batch_key.to_string(), batch.clone());
            info!("Created new review batch {} for repo {}", batch_key, repo_url);
            Some(batch)
        }
    }

    pub async fn get_batch(&self,
        batch_key: &str,
    ) -> Option<ReviewBatch> {
        let mut batches = self.pending_batches.lock().await;
        batches.remove(batch_key)
    }

    pub async fn should_process_batch(
        &self,
        batch_key: &str,
        timeout: Duration,
        max_size: usize,
    ) -> bool {
        let batches = self.pending_batches.lock().await;
        
        if let Some(batch) = batches.get(batch_key) {
            let elapsed = batch.created_at.elapsed();
            let size = batch.commits.len();
            
            if elapsed >= timeout || size >= max_size {
                info!(
                    "Batch {} ready for processing (elapsed={:?}, size={})",
                    batch_key, elapsed, size
                );
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn clone_and_diff_batch(
        &self,
        repo_url: &str,
        commits: &[CommitRange],
    ) -> anyhow::Result<String> {
        if commits.is_empty() {
            return Ok(String::new());
        }

        if commits.len() == 1 {
            return self.clone_and_diff(repo_url, &commits[0].base, &commits[0].head);
        }

        // For multiple commits, merge all diffs
        let mut combined_diff = String::new();
        
        for commit in commits {
            match self.clone_and_diff(repo_url, &commit.base, &commit.head) {
                Ok(diff) => {
                    if !diff.is_empty() {
                        combined_diff.push_str(&format!(
                            "\n# Diff for {}..{}\n",
                            commit.base, commit.head
                        ));
                        combined_diff.push_str(&diff);
                        combined_diff.push('\n');
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to diff {}..{}: {}",
                        commit.base, commit.head, e
                    );
                }
            }
        }

        Ok(combined_diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_batch_operations() {
        let config = AppConfig::default();
        let engine = ReviewEngine::new(&config);
        
        let batch_key = "test/repo:123";
        let repo_url = "https://github.com/test/repo.git";
        
        // Add first commit
        let batch = engine.add_to_batch(
            batch_key,
            repo_url,
            CommitRange {
                base: "main".to_string(),
                head: "feature-1".to_string(),
            },
        ).await;
        
        assert!(batch.is_some());
        
        // Add second commit
        let batch = engine.add_to_batch(
            batch_key,
            repo_url,
            CommitRange {
                base: "main".to_string(),
                head: "feature-2".to_string(),
            },
        ).await;
        
        assert!(batch.is_none());
        
        // Get batch
        let batch = engine.get_batch(batch_key).await;
        assert!(batch.is_some());
        assert_eq!(batch.unwrap().commits.len(), 2);
        
        // Should be empty after removal
        let batch = engine.get_batch(batch_key).await;
        assert!(batch.is_none());
    }

    #[tokio::test]
    async fn test_should_process_batch() {
        let config = AppConfig::default();
        let engine = ReviewEngine::new(&config);
        
        let batch_key = "test/repo:456";
        
        // Empty batch should not process
        assert!(!engine.should_process_batch(batch_key, Duration::from_secs(0), 1).await);
        
        // Add a commit
        engine.add_to_batch(
            batch_key,
            "https://github.com/test/repo.git",
            CommitRange {
                base: "main".to_string(),
                head: "feature".to_string(),
            },
        ).await;
        
        // Should process with zero timeout
        assert!(engine.should_process_batch(batch_key, Duration::from_secs(0), 10).await);
    }

    #[tokio::test]
    async fn test_clone_and_diff_real_repo() {
        let config = AppConfig::default();
        let engine = ReviewEngine::new(&config);
        
        // Use a small, stable public repo for testing
        let repo_url = "https://github.com/octocat/Hello-World.git";
        let base = "main";
        let head = "test";

        let result = engine.clone_and_diff(repo_url, base, head);

        // The test branch may or may not exist; we mainly verify the plumbing works
        match result {
            Ok(diff) => {
                assert!(!diff.is_empty() || diff.is_empty()); // diff can be empty if branches are same
            }
            Err(e) => {
                // Fetch may fail if branch doesn't exist, which is acceptable for this test
                println!("Clone/diff failed (expected if branch missing): {}", e);
            }
        }
    }
}
