use tracing::info;

/// Detects trivial PRs that can be auto-approved without LLM review.
pub struct AutoApprover;

#[derive(Debug, Clone, Default)]
pub struct AutoApproveConfig {
    pub enabled: bool,
    pub docs_patterns: Vec<String>,
    pub skip_lockfiles: bool,
    pub skip_whitespace: bool,
}

impl Default for AutoApprover {
    fn default() -> Self {
        Self
    }
}

impl AutoApprover {
    pub fn new() -> Self {
        Self
    }

    pub fn is_trivial(diff: &str, config: &AutoApproveConfig) -> Option<String> {
        if !config.enabled {
            return None;
        }

        if diff.trim().is_empty() {
            return Some("empty diff".to_string());
        }

        // Check if all changes are whitespace-only
        if config.skip_whitespace && is_whitespace_only(diff) {
            return Some("whitespace-only changes".to_string());
        }

        // Check if all changed files are lockfiles
        if config.skip_lockfiles && is_lockfile_only(diff) {
            return Some("lockfile-only changes".to_string());
        }

        // Check if all changed files match documentation patterns
        if !config.docs_patterns.is_empty() && is_docs_only(diff, &config.docs_patterns) {
            return Some("documentation-only changes".to_string());
        }

        None
    }
}

fn is_whitespace_only(diff: &str) -> bool {
    let mut has_changes = false;

    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            has_changes = true;
            let trimmed = line[1..].trim();
            if !trimmed.is_empty() {
                return false;
            }
        }
        if line.starts_with('-') && !line.starts_with("---") {
            has_changes = true;
            let trimmed = line[1..].trim();
            if !trimmed.is_empty() {
                return false;
            }
        }
    }

    has_changes
}

fn is_lockfile_only(diff: &str) -> bool {
    let lockfile_names = [
        "Cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "Pipfile.lock",
        "poetry.lock",
        "go.sum",
        "Gemfile.lock",
        "composer.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
    ];

    let mut has_files = false;
    let mut all_lockfiles = true;

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            has_files = true;
            let file_path = extract_file_path(line).unwrap_or_default();
            let is_lockfile = lockfile_names.iter().any(|&name| file_path.ends_with(name));
            if !is_lockfile {
                all_lockfiles = false;
                break;
            }
        }
    }

    has_files && all_lockfiles
}

fn is_docs_only(diff: &str, patterns: &[String]) -> bool {
    let mut has_files = false;
    let mut all_docs = true;

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            has_files = true;
            let file_path = extract_file_path(line).unwrap_or_default();
            let is_doc = patterns.iter().any(|pat| {
                if let Some(suffix) = pat.strip_prefix('*') {
                    file_path.ends_with(suffix)
                } else {
                    file_path.contains(pat)
                }
            });
            if !is_doc {
                all_docs = false;
                break;
            }
        }
    }

    has_files && all_docs
}

fn extract_file_path(diff_line: &str) -> Option<String> {
    let parts: Vec<&str> = diff_line.split_whitespace().collect();
    if parts.len() >= 4 {
        let path = parts[2].strip_prefix("a/").unwrap_or(parts[2]);
        Some(path.to_string())
    } else {
        None
    }
}

pub fn auto_approve_message(reason: &str) -> String {
    info!("Auto-approving PR: {}", reason);
    format!("\u{2705} Auto-approved: {}", reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whitespace_only() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-
+
 }
"#;
        assert!(is_whitespace_only(diff));
    }

    #[test]
    fn test_not_whitespace_only() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    let x = 1;
+    let x = 2;
 }
"#;
        assert!(!is_whitespace_only(diff));
    }

    #[test]
    fn test_lockfile_only() {
        let diff = r#"diff --git a/Cargo.lock b/Cargo.lock
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,5 +1,5 @@
 version = 1
-diff = old
+diff = new
"#;
        assert!(is_lockfile_only(diff));
    }

    #[test]
    fn test_not_lockfile_only() {
        let diff = r#"diff --git a/Cargo.lock b/Cargo.lock
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,5 +1,5 @@
 version = 1

diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {}
"#;
        assert!(!is_lockfile_only(diff));
    }

    #[test]
    fn test_docs_only() {
        let diff = r#"diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1,5 +1,5 @@
 # Project
-Old
+New
"#;
        let patterns = vec!["README".to_string(), "*.md".to_string()];
        assert!(is_docs_only(diff, &patterns));
    }

    #[test]
    fn test_auto_approve_disabled() {
        let config = AutoApproveConfig {
            enabled: false,
            ..Default::default()
        };
        let diff = "diff --git a/Cargo.lock b/Cargo.lock\n";
        assert!(AutoApprover::is_trivial(diff, &config).is_none());
    }

    #[test]
    fn test_auto_approve_whitespace() {
        let config = AutoApproveConfig {
            enabled: true,
            docs_patterns: vec![],
            skip_lockfiles: false,
            skip_whitespace: true,
        };

        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-
+
 }
"#;

        let result = AutoApprover::is_trivial(diff, &config);
        assert!(result.is_some());
        assert!(result.unwrap().contains("whitespace"));
    }

    #[test]
    fn test_auto_approve_lockfile() {
        let config = AutoApproveConfig {
            enabled: true,
            docs_patterns: vec![],
            skip_lockfiles: true,
            skip_whitespace: false,
        };

        let diff = r#"diff --git a/Cargo.lock b/Cargo.lock
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,5 +1,5 @@
 version = 1
"#;

        let result = AutoApprover::is_trivial(diff, &config);
        assert!(result.is_some());
        assert!(result.unwrap().contains("lockfile"));
    }

    #[test]
    fn test_auto_approve_docs() {
        let config = AutoApproveConfig {
            enabled: true,
            docs_patterns: vec!["*.md".to_string(), "README".to_string()],
            skip_lockfiles: false,
            skip_whitespace: false,
        };

        let diff = r#"diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1,5 +1,5 @@
 # Project
-Old
+New
"#;

        let result = AutoApprover::is_trivial(diff, &config);
        assert!(result.is_some());
        assert!(result.unwrap().contains("documentation"));
    }

    #[test]
    fn test_auto_approve_message() {
        let msg = auto_approve_message("trivial changes");
        assert!(msg.contains("Auto-approved"));
        assert!(msg.contains("trivial changes"));
    }
}
