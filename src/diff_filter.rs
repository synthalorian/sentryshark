use regex::Regex;
use tracing::{debug, info};

pub struct DiffFilter {
    lockfile_patterns: Vec<Regex>,
    generated_patterns: Vec<Regex>,
    include_patterns: Vec<Regex>,
    exclude_patterns: Vec<Regex>,
    enabled: bool,
}

impl DiffFilter {
    pub fn new(
        lockfile_patterns: &[String],
        generated_patterns: &[String],
        enabled: bool,
    ) -> Self {
        Self::with_patterns(
            lockfile_patterns,
            generated_patterns,
            &[],
            &[],
            enabled,
        )
    }

    pub fn with_patterns(
        lockfile_patterns: &[String],
        generated_patterns: &[String],
        include_patterns: &[String],
        exclude_patterns: &[String],
        enabled: bool,
    ) -> Self {
        let lockfile_patterns = compile_patterns(lockfile_patterns);
        let generated_patterns = compile_patterns(generated_patterns);
        let include_patterns = compile_patterns(include_patterns);
        let exclude_patterns = compile_patterns(exclude_patterns);
        
        info!(
            "DiffFilter initialized with {} lockfile, {} generated, {} include, {} exclude patterns (enabled={})",
            lockfile_patterns.len(),
            generated_patterns.len(),
            include_patterns.len(),
            exclude_patterns.len(),
            enabled
        );
        
        Self {
            lockfile_patterns,
            generated_patterns,
            include_patterns,
            exclude_patterns,
            enabled,
        }
    }

    pub fn filter_diff(&self, diff: &str) -> String {
        if !self.enabled {
            return diff.to_string();
        }

        let mut result = String::new();
        let mut current_file: Option<String> = None;
        let mut skip_current_file = false;
        let mut current_hunk = String::new();
        let mut filtered_count = 0;
        let mut kept_count = 0;

        for line in diff.lines() {
            if line.starts_with("diff --git") {
                // Process previous file's hunk if any
                if let Some(ref file) = current_file {
                    if !skip_current_file {
                        result.push_str(&current_hunk);
                        kept_count += 1;
                    } else {
                        filtered_count += 1;
                        debug!("Filtered out file: {}", file);
                    }
                }
                
                current_hunk = String::new();
                current_hunk.push_str(line);
                current_hunk.push('\n');
                
                // Extract file path from diff --git a/path b/path
                if let Some(file_path) = extract_file_path(line) {
                    skip_current_file = self.should_skip(&file_path);
                    current_file = Some(file_path);
                } else {
                    skip_current_file = false;
                    current_file = None;
                }
            } else {
                current_hunk.push_str(line);
                current_hunk.push('\n');
            }
        }

        // Process last file
        if let Some(ref file) = current_file {
            if !skip_current_file {
                result.push_str(&current_hunk);
                kept_count += 1;
            } else {
                filtered_count += 1;
                debug!("Filtered out file: {}", file);
            }
        }

        info!(
            "Diff filtering complete: {} files kept, {} files filtered",
            kept_count, filtered_count
        );

        result
    }

    fn should_skip(&self, file_path: &str) -> bool {
        let path = file_path.to_lowercase();
        
        // If include patterns are specified, only include matching files
        if !self.include_patterns.is_empty() {
            let included = self.include_patterns.iter().any(|p| p.is_match(&path));
            if !included {
                return true;
            }
        }
        
        // Check exclude patterns
        for pattern in &self.exclude_patterns {
            if pattern.is_match(&path) {
                return true;
            }
        }
        
        // Check lockfile patterns
        for pattern in &self.lockfile_patterns {
            if pattern.is_match(&path) {
                return true;
            }
        }
        
        // Check generated patterns
        for pattern in &self.generated_patterns {
            if pattern.is_match(&path) {
                return true;
            }
        }
        
        false
    }
}

fn compile_patterns(patterns: &[String]) -> Vec<Regex> {
    patterns
        .iter()
        .filter_map(|p| {
            // Convert glob-like patterns to regex
            let regex_pattern = glob_to_regex(p);
            match Regex::new(&regex_pattern) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!("Failed to compile pattern '{}': {}", p, e);
                    None
                }
            }
        })
        .collect()
}

fn glob_to_regex(pattern: &str) -> String {
    let mut regex = String::new();
    regex.push('^');
    
    let trailing_slash = pattern.ends_with('/');
    let pattern_without_trailing_slash = if trailing_slash {
        &pattern[..pattern.len() - 1]
    } else {
        pattern
    };
    
    for ch in pattern_without_trailing_slash.chars() {
        match ch {
            '\\' => regex.push_str("\\\\"),
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' => regex.push_str("\\."),
            '/' => regex.push('/'),
            '+' => regex.push_str("\\+"),
            '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    
    if trailing_slash {
        regex.push_str("/.*$");
    } else {
        regex.push('$');
    }
    
    regex
}

fn extract_file_path(diff_line: &str) -> Option<String> {
    // diff --git a/path/to/file b/path/to/file
    let parts: Vec<&str> = diff_line.split_whitespace().collect();
    if parts.len() >= 4 {
        let path = parts[2].strip_prefix("a/").unwrap_or(parts[2]);
        Some(path.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_lockfiles() {
        let filter = DiffFilter::new(
            &["Cargo.lock".to_string(), "*.lock".to_string()],
            &["dist/".to_string()],
            true,
        );

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

        let filtered = filter.filter_diff(diff);
        assert!(!filtered.contains("Cargo.lock"), "Cargo.lock should be filtered out");
        assert!(filtered.contains("src/main.rs"), "src/main.rs should be kept, got: {filtered}");
    }

    #[test]
    fn test_filter_generated() {
        let filter = DiffFilter::new(
            &[],
            &["*.min.js".to_string(), "dist/".to_string()],
            true,
        );

        let diff = r#"diff --git a/dist/bundle.min.js b/dist/bundle.min.js
--- a/dist/bundle.min.js
+++ b/dist/bundle.min.js
@@ -1,5 +1,5 @@
 console.log("minified")
diff --git a/src/app.js b/src/app.js
--- a/src/app.js
+++ b/src/app.js
@@ -1,5 +1,5 @@
 console.log("app")
"#;

        let filtered = filter.filter_diff(diff);
        assert!(!filtered.contains("bundle.min.js"));
        assert!(filtered.contains("src/app.js"));
    }

    #[test]
    fn test_disabled_filter() {
        let filter = DiffFilter::new(
            &["Cargo.lock".to_string()],
            &[],
            false,
        );

        let diff = "diff --git a/Cargo.lock b/Cargo.lock\n--- a/Cargo.lock\n+++ b/Cargo.lock\n";
        let filtered = filter.filter_diff(diff);
        assert!(filtered.contains("Cargo.lock"));
    }

    #[test]
    fn test_glob_to_regex() {
        assert_eq!(glob_to_regex("*.lock"), "^.*\\.lock$");
        assert_eq!(glob_to_regex("dist/"), "^dist/.*$");
        assert_eq!(glob_to_regex("Cargo.lock"), "^Cargo\\.lock$");
    }

    #[test]
    fn test_include_patterns() {
        let filter = DiffFilter::with_patterns(
            &[],
            &[],
            &["src/*".to_string()],
            &[],
            true,
        );

        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {}
diff --git a/tests/test.rs b/tests/test.rs
--- a/tests/test.rs
+++ b/tests/test.rs
@@ -1,5 +1,5 @@
 fn test() {}
"#;

        let filtered = filter.filter_diff(diff);
        assert!(filtered.contains("src/main.rs"), "src/main.rs should be kept");
        assert!(!filtered.contains("tests/test.rs"), "tests/test.rs should be filtered out");
    }

    #[test]
    fn test_exclude_patterns() {
        let filter = DiffFilter::with_patterns(
            &[],
            &[],
            &[],
            &["tests/*".to_string(), "*.test.js".to_string()],
            true,
        );

        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {}
diff --git a/tests/test.rs b/tests/test.rs
--- a/tests/test.rs
+++ b/tests/test.rs
@@ -1,5 +1,5 @@
 fn test() {}
"#;

        let filtered = filter.filter_diff(diff);
        assert!(filtered.contains("src/main.rs"), "src/main.rs should be kept");
        assert!(!filtered.contains("tests/test.rs"), "tests/test.rs should be filtered out");
    }

    #[test]
    fn test_include_and_exclude_combined() {
        let filter = DiffFilter::with_patterns(
            &[],
            &[],
            &["src/*".to_string()],
            &["src/generated.rs".to_string()],
            true,
        );

        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {}
diff --git a/src/generated.rs b/src/generated.rs
--- a/src/generated.rs
+++ b/src/generated.rs
@@ -1,5 +1,5 @@
 // generated
diff --git a/lib/lib.rs b/lib/lib.rs
--- a/lib/lib.rs
+++ b/lib/lib.rs
@@ -1,5 +1,5 @@
 pub fn lib() {}
"#;

        let filtered = filter.filter_diff(diff);
        assert!(filtered.contains("src/main.rs"), "src/main.rs should be kept");
        assert!(!filtered.contains("src/generated.rs"), "src/generated.rs should be excluded");
        assert!(!filtered.contains("lib/lib.rs"), "lib/lib.rs should not be included");
    }

    #[test]
    fn test_glob_to_regex_escapes_backslash() {
        // Backslash should be escaped in regex
        assert_eq!(glob_to_regex(r"path\to\file"), r"^path\\to\\file$");
    }
}
