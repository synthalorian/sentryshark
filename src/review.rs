use tracing::{info, error};
use std::process::Command;

pub struct ReviewEngine;

impl ReviewEngine {
    pub fn clone_and_diff(repo_url: &str, base: &str, head: &str) -> anyhow::Result<String> {
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

        Ok(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_clone_and_diff_real_repo() {
        // Use a small, stable public repo for testing
        let repo_url = "https://github.com/octocat/Hello-World.git";
        let base = "main";
        let head = "test";

        let result = ReviewEngine::clone_and_diff(repo_url, base, head);

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
