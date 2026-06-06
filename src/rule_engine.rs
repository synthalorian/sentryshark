use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SeverityLevel {
    Critical,
    Warning,
    Info,
}

impl std::fmt::Display for SeverityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeverityLevel::Critical => write!(f, "critical"),
            SeverityLevel::Warning => write!(f, "warning"),
            SeverityLevel::Info => write!(f, "info"),
        }
    }
}

impl std::str::FromStr for SeverityLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "critical" => Ok(SeverityLevel::Critical),
            "warning" => Ok(SeverityLevel::Warning),
            "info" => Ok(SeverityLevel::Info),
            _ => Err(format!("Unknown severity level: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRule {
    pub name: String,
    pub description: String,
    pub pattern: String,
    pub severity: SeverityLevel,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct RuleEngine {
    rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone)]
struct CompiledRule {
    rule: ReviewRule,
    regex: Regex,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleMatch {
    pub rule_name: String,
    pub severity: SeverityLevel,
    pub message: String,
    pub file_path: Option<String>,
    pub line: Option<u32>,
}

impl RuleEngine {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn load_from_directory(dir: &str) -> anyhow::Result<Self> {
        let mut engine = Self::new();
        let path = Path::new(dir);

        if !path.exists() {
            info!("Rules directory '{}' does not exist, skipping", dir);
            return Ok(engine);
        }

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("yaml")
                || path.extension().and_then(|s| s.to_str()) == Some("yml")
            {
                match Self::load_rules_from_file(&path) {
                    Ok(rules) => {
                        info!("Loaded {} rules from {:?}", rules.len(), path);
                        for rule in rules {
                            if let Err(e) = engine.add_rule(rule) {
                                warn!("Failed to compile rule: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to load rules from {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(engine)
    }

    pub fn load_rules_from_file(path: &Path) -> anyhow::Result<Vec<ReviewRule>> {
        let content = std::fs::read_to_string(path)?;
        let rules: Vec<ReviewRule> = serde_yaml::from_str(&content)?;
        Ok(rules)
    }

    pub fn add_rule(&mut self, rule: ReviewRule) -> anyhow::Result<()> {
        let regex = Regex::new(&rule.pattern).map_err(|e| {
            anyhow::anyhow!("Invalid regex pattern for rule '{}': {}", rule.name, e)
        })?;
        self.rules.push(CompiledRule { rule, regex });
        Ok(())
    }

    pub fn add_rules(&mut self, rules: Vec<ReviewRule>) {
        for rule in rules {
            if let Err(e) = self.add_rule(rule) {
                warn!("Failed to add rule: {}", e);
            }
        }
    }

    pub fn check_diff(&self, diff: &str) -> Vec<RuleMatch> {
        let mut matches = Vec::new();
        let mut current_file: Option<String> = None;
        let mut current_line: u32 = 0;

        for line in diff.lines() {
            if line.starts_with("diff --git") {
                current_file = extract_file_path(line);
                current_line = 0;
                continue;
            }

            if line.starts_with("@@") {
                if let Some(l) = parse_hunk_start_line(line) {
                    current_line = l;
                }
                continue;
            }

            if line.starts_with('+') && !line.starts_with("+++") {
                current_line += 1;
                for compiled in &self.rules {
                    if compiled.regex.is_match(line) {
                        debug!(
                            "Rule '{}' matched in {}:{}",
                            compiled.rule.name,
                            current_file.as_deref().unwrap_or("unknown"),
                            current_line
                        );
                        matches.push(RuleMatch {
                            rule_name: compiled.rule.name.clone(),
                            severity: compiled.rule.severity.clone(),
                            message: compiled.rule.message.clone(),
                            file_path: current_file.clone(),
                            line: Some(current_line),
                        });
                    }
                }
            } else if !line.starts_with('-') && !line.starts_with("---") {
                current_line += 1;
            }
        }

        matches
    }

    pub fn check_content(&self, content: &str) -> Vec<RuleMatch> {
        let mut matches = Vec::new();
        for line in content.lines() {
            for compiled in &self.rules {
                if compiled.regex.is_match(line) {
                    matches.push(RuleMatch {
                        rule_name: compiled.rule.name.clone(),
                        severity: compiled.rule.severity.clone(),
                        message: compiled.rule.message.clone(),
                        file_path: None,
                        line: None,
                    });
                }
            }
        }
        matches
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn format_rules_for_prompt(&self) -> String {
        if self.rules.is_empty() {
            return String::new();
        }

        let mut prompt = String::from("Additional custom rules to check:\n");
        for compiled in &self.rules {
            prompt.push_str(&format!(
                "- [{}] {}: {}\n",
                compiled.rule.severity, compiled.rule.name, compiled.rule.description
            ));
        }
        prompt.push('\n');
        prompt
    }
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

fn parse_hunk_start_line(hunk_line: &str) -> Option<u32> {
    // @@ -old_start,old_count +new_start,new_count @@
    let start = hunk_line.find('+')?;
    let end = hunk_line[start..].find(',').or_else(|| hunk_line[start..].find(' '))?;
    let num_str = &hunk_line[start + 1..start + end];
    num_str.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_from_str() {
        assert_eq!(
            "critical".parse::<SeverityLevel>().unwrap(),
            SeverityLevel::Critical
        );
        assert_eq!(
            "warning".parse::<SeverityLevel>().unwrap(),
            SeverityLevel::Warning
        );
        assert_eq!("info".parse::<SeverityLevel>().unwrap(), SeverityLevel::Info);
        assert!("unknown".parse::<SeverityLevel>().is_err());
    }

    #[test]
    fn test_add_rule_and_check() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(ReviewRule {
                name: "no_unwrap".to_string(),
                description: "Avoid unwrap in production code".to_string(),
                pattern: r"unwrap\(\)".to_string(),
                severity: SeverityLevel::Warning,
                message: "Consider using unwrap_or or expect with a message".to_string(),
            })
            .unwrap();

        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    let x = Some(1);
+    let x = Some(1).unwrap();
 }
"#;

        let matches = engine.check_diff(diff);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rule_name, "no_unwrap");
        assert_eq!(matches[0].severity, SeverityLevel::Warning);
        assert_eq!(matches[0].file_path, Some("src/main.rs".to_string()));
        assert_eq!(matches[0].line, Some(3));
    }

    #[test]
    fn test_check_content() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(ReviewRule {
                name: "no_todo".to_string(),
                description: "TODO items should be tracked".to_string(),
                pattern: r"TODO|FIXME".to_string(),
                severity: SeverityLevel::Info,
                message: "Consider creating an issue for this TODO".to_string(),
            })
            .unwrap();

        let content = "// TODO: refactor this\nfn main() {}";
        let matches = engine.check_content(content);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rule_name, "no_todo");
    }

    #[test]
    fn test_empty_engine() {
        let engine = RuleEngine::new();
        assert!(engine.is_empty());
        assert_eq!(engine.len(), 0);

        let diff = "diff --git a/src/main.rs b/src/main.rs\n+unwrap();";
        let matches = engine.check_diff(diff);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_invalid_regex() {
        let mut engine = RuleEngine::new();
        let result = engine.add_rule(ReviewRule {
            name: "bad".to_string(),
            description: "Invalid regex".to_string(),
            pattern: r"[invalid".to_string(),
            severity: SeverityLevel::Info,
            message: "test".to_string(),
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_format_rules_for_prompt() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(ReviewRule {
                name: "rule1".to_string(),
                description: "Description 1".to_string(),
                pattern: r"pattern1".to_string(),
                severity: SeverityLevel::Critical,
                message: "msg1".to_string(),
            })
            .unwrap();

        let prompt = engine.format_rules_for_prompt();
        assert!(prompt.contains("Additional custom rules"));
        assert!(prompt.contains("rule1"));
        assert!(prompt.contains("Description 1"));
    }

    #[test]
    fn test_parse_hunk_start_line() {
        assert_eq!(parse_hunk_start_line("@@ -1,5 +10,7 @@"), Some(10));
        assert_eq!(parse_hunk_start_line("@@ -0,0 +1 @@"), Some(1));
        assert_eq!(parse_hunk_start_line("@@ -5 +10 @@"), Some(10));
    }

    #[test]
    fn test_multiple_rules() {
        let mut engine = RuleEngine::new();
        engine
            .add_rule(ReviewRule {
                name: "no_unwrap".to_string(),
                description: "No unwrap".to_string(),
                pattern: r"unwrap\(\)".to_string(),
                severity: SeverityLevel::Warning,
                message: "Avoid unwrap".to_string(),
            })
            .unwrap();
        engine
            .add_rule(ReviewRule {
                name: "no_panic".to_string(),
                description: "No panic".to_string(),
                pattern: r"panic!".to_string(),
                severity: SeverityLevel::Critical,
                message: "Avoid panic".to_string(),
            })
            .unwrap();

        let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,5 @@
 fn main() {
-    let x = Some(1);
+    let x = Some(1).unwrap();
+    panic!("oh no");
 }
"#;

        let matches = engine.check_diff(diff);
        assert_eq!(matches.len(), 2);
        let severities: Vec<_> = matches.iter().map(|m| &m.severity).collect();
        assert!(severities.contains(&&SeverityLevel::Warning));
        assert!(severities.contains(&&SeverityLevel::Critical));
    }
}
