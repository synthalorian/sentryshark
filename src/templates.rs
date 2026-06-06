/// Project-specific review templates.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ProjectTemplate {
    Rust,
    Python,
    JavaScript,
    #[default]
    Generic,
}

impl std::fmt::Display for ProjectTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectTemplate::Rust => write!(f, "rust"),
            ProjectTemplate::Python => write!(f, "python"),
            ProjectTemplate::JavaScript => write!(f, "javascript"),
            ProjectTemplate::Generic => write!(f, "generic"),
        }
    }
}

impl std::str::FromStr for ProjectTemplate {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "rust" | "rs" => Ok(ProjectTemplate::Rust),
            "python" | "py" => Ok(ProjectTemplate::Python),
            "javascript" | "js" | "typescript" | "ts" => Ok(ProjectTemplate::JavaScript),
            "generic" | "" => Ok(ProjectTemplate::Generic),
            _ => Err(format!("Unknown project template: {}", s)),
        }
    }
}

pub struct TemplateEngine;

impl TemplateEngine {
    pub fn prompt_additions(template: &ProjectTemplate) -> String {
        match template {
            ProjectTemplate::Rust => Self::rust_prompt(),
            ProjectTemplate::Python => Self::python_prompt(),
            ProjectTemplate::JavaScript => Self::javascript_prompt(),
            ProjectTemplate::Generic => String::new(),
        }
    }

    fn rust_prompt() -> String {
        r#"
Rust-specific review guidelines:
- Avoid unwrap() and expect() in production code; use Result propagation
- Check for unnecessary Clone usage; prefer borrowing
- Review unsafe blocks carefully; ensure soundness and document invariants
- Verify Send/Sync implementations for thread safety
- Check for potential panics in indexing (e.g., slice[i] without bounds check)
- Prefer match over if let Some(_) when all variants should be handled
- Check for proper error types and meaningful error messages
- Review async code for cancellation safety
"#
        .to_string()
    }

    fn python_prompt() -> String {
        r#"
Python-specific review guidelines:
- Check for type hints and mypy compatibility
- Review exception handling; avoid bare except clauses
- Check for mutable default arguments
- Verify proper resource cleanup (context managers, try/finally)
- Check for potential SQL injection or command injection
- Review for race conditions in concurrent code
- Check for proper string formatting (avoid f-strings with user input in logging)
- Verify test coverage for edge cases
"#
        .to_string()
    }

    fn javascript_prompt() -> String {
        r#"
JavaScript/TypeScript-specific review guidelines:
- Check for proper async/await usage; avoid callback hell
- Review for potential XSS vulnerabilities in DOM manipulation
- Check for proper error handling in Promise chains
- Verify TypeScript strict mode compliance
- Check for memory leaks (event listeners, subscriptions)
- Review for proper input validation
- Check for race conditions in state management
- Verify proper use of === vs ==
"#
        .to_string()
    }

    pub fn detect_from_files(file_paths: &[&str]) -> ProjectTemplate {
        let mut rust_count = 0;
        let mut python_count = 0;
        let mut js_count = 0;

        for path in file_paths {
            let lower = path.to_lowercase();
            if lower.ends_with(".rs") || lower.contains("cargo.toml") {
                rust_count += 1;
            } else if lower.ends_with(".py")
                || lower.contains("requirements.txt")
                || lower.contains("pyproject.toml")
            {
                python_count += 1;
            } else if lower.ends_with(".js")
                || lower.ends_with(".ts")
                || lower.ends_with(".jsx")
                || lower.ends_with(".tsx")
                || lower.contains("package.json")
            {
                js_count += 1;
            }
        }

        if rust_count >= python_count && rust_count >= js_count && rust_count > 0 {
            ProjectTemplate::Rust
        } else if python_count >= rust_count && python_count >= js_count && python_count > 0 {
            ProjectTemplate::Python
        } else if js_count > 0 {
            ProjectTemplate::JavaScript
        } else {
            ProjectTemplate::Generic
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_from_str() {
        assert_eq!(
            "rust".parse::<ProjectTemplate>().unwrap(),
            ProjectTemplate::Rust
        );
        assert_eq!(
            "python".parse::<ProjectTemplate>().unwrap(),
            ProjectTemplate::Python
        );
        assert_eq!(
            "javascript".parse::<ProjectTemplate>().unwrap(),
            ProjectTemplate::JavaScript
        );
        assert_eq!(
            "generic".parse::<ProjectTemplate>().unwrap(),
            ProjectTemplate::Generic
        );
        assert!("unknown".parse::<ProjectTemplate>().is_err());
    }

    #[test]
    fn test_rust_prompt() {
        let prompt = TemplateEngine::prompt_additions(&ProjectTemplate::Rust);
        assert!(prompt.contains("unwrap()"));
        assert!(prompt.contains("unsafe"));
        assert!(prompt.contains("Clone"));
    }

    #[test]
    fn test_python_prompt() {
        let prompt = TemplateEngine::prompt_additions(&ProjectTemplate::Python);
        assert!(prompt.contains("type hints"));
        assert!(prompt.contains("mutable default"));
    }

    #[test]
    fn test_javascript_prompt() {
        let prompt = TemplateEngine::prompt_additions(&ProjectTemplate::JavaScript);
        assert!(prompt.contains("async/await"));
        assert!(prompt.contains("XSS"));
    }

    #[test]
    fn test_generic_prompt() {
        let prompt = TemplateEngine::prompt_additions(&ProjectTemplate::Generic);
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_detect_from_files() {
        let files = vec!["src/main.rs", "Cargo.toml", "src/lib.rs"];
        assert_eq!(
            TemplateEngine::detect_from_files(&files),
            ProjectTemplate::Rust
        );

        let files = vec!["main.py", "requirements.txt"];
        assert_eq!(
            TemplateEngine::detect_from_files(&files),
            ProjectTemplate::Python
        );

        let files = vec!["src/app.js", "package.json"];
        assert_eq!(
            TemplateEngine::detect_from_files(&files),
            ProjectTemplate::JavaScript
        );

        let files = vec!["README.md", "LICENSE"];
        assert_eq!(
            TemplateEngine::detect_from_files(&files),
            ProjectTemplate::Generic
        );
    }

    #[test]
    fn test_template_display() {
        assert_eq!(ProjectTemplate::Rust.to_string(), "rust");
        assert_eq!(ProjectTemplate::Python.to_string(), "python");
        assert_eq!(ProjectTemplate::JavaScript.to_string(), "javascript");
        assert_eq!(ProjectTemplate::Generic.to_string(), "generic");
    }
}
