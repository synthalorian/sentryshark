use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub github: Option<GitHubConfig>,
    pub gitlab: Option<GitLabConfig>,
    pub llm: LlmConfig,
    pub review: Option<ReviewConfig>,
    pub diff_filter: Option<DiffFilterConfig>,
    pub batching: Option<BatchingConfig>,
    pub database: Option<DatabaseConfig>,
    pub dashboard: Option<DashboardConfig>,
    pub auto_approve: Option<AutoApproveConfig>,
    pub retry: Option<RetryConfig>,
    pub queue: Option<QueueConfig>,
    pub cache: Option<CacheConfig>,
    pub rules: Option<RulesConfig>,
    pub logging: Option<LoggingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    pub webhook_secret: String,
    pub app_id: String,
    pub private_key_path: String,
    #[serde(default = "default_false")]
    pub use_app_auth: bool,
    #[serde(default)]
    pub installation_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabConfig {
    pub webhook_secret: String,
    pub access_token: String,
    #[serde(default = "default_false")]
    pub ci_cd_enabled: bool,
    #[serde(default = "default_gitlab_url")]
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String, // "llamacpp", "ollama", "vllm"
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    #[serde(default = "default_true")]
    pub security: bool,
    #[serde(default = "default_true")]
    pub style: bool,
    #[serde(default = "default_true")]
    pub performance: bool,
    #[serde(default = "default_true")]
    pub correctness: bool,
    #[serde(default = "default_true")]
    pub maintainability: bool,
    #[serde(default)]
    pub inline_comments: bool,
    #[serde(default)]
    pub summary_comment: bool,
    #[serde(default)]
    pub template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffFilterConfig {
    #[serde(default = "default_lockfile_patterns")]
    pub lockfile_patterns: Vec<String>,
    #[serde(default = "default_generated_patterns")]
    pub generated_patterns: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchingConfig {
    #[serde(default = "default_batching_enabled")]
    pub enabled: bool,
    #[serde(default = "default_batch_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_batch_max_size")]
    pub max_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoApproveConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_docs_patterns")]
    pub docs_patterns: Vec<String>,
    #[serde(default = "default_true")]
    pub skip_lockfiles: bool,
    #[serde(default = "default_true")]
    pub skip_whitespace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_base_delay_ms")]
    pub base_delay_ms: u64,
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    #[serde(default = "default_worker_count")]
    pub worker_count: usize,
    #[serde(default = "default_concurrency_limit")]
    pub concurrency_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cache_ttl_hours")]
    pub ttl_hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesConfig {
    #[serde(default = "default_rules_dir")]
    pub rules_dir: String,
    #[serde(default)]
    pub inline_rules: Vec<crate::rule_engine::ReviewRule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_json_logging")]
    pub json_format: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_dashboard_refresh")]
    pub refresh_seconds: u64,
}

fn default_true() -> bool {
    true
}

fn default_batching_enabled() -> bool {
    false
}

fn default_batch_timeout_seconds() -> u64 {
    30
}

fn default_batch_max_size() -> usize {
    10
}

fn default_false() -> bool {
    false
}

fn default_gitlab_url() -> String {
    "https://gitlab.com".to_string()
}

fn default_db_path() -> String {
    "sentryshark.db".to_string()
}

fn default_dashboard_refresh() -> u64 {
    30
}

fn default_docs_patterns() -> Vec<String> {
    vec![
        "*.md".to_string(),
        "README".to_string(),
        "CHANGELOG".to_string(),
        "LICENSE".to_string(),
        "CONTRIBUTING".to_string(),
        "docs/".to_string(),
    ]
}

fn default_max_retries() -> u32 {
    3
}

fn default_base_delay_ms() -> u64 {
    1000
}

fn default_max_delay_ms() -> u64 {
    30000
}

fn default_worker_count() -> usize {
    4
}

fn default_concurrency_limit() -> usize {
    1
}

fn default_cache_enabled() -> bool {
    true
}

fn default_cache_ttl_hours() -> u64 {
    24
}

fn default_rules_dir() -> String {
    "rules".to_string()
}

fn default_json_logging() -> bool {
    false
}

fn default_lockfile_patterns() -> Vec<String> {
    vec![
        "Cargo.lock".to_string(),
        "package-lock.json".to_string(),
        "yarn.lock".to_string(),
        "Pipfile.lock".to_string(),
        "poetry.lock".to_string(),
        "go.sum".to_string(),
        "Gemfile.lock".to_string(),
        "composer.lock".to_string(),
        "*.lock".to_string(),
    ]
}

fn default_generated_patterns() -> Vec<String> {
    vec![
        "*.min.js".to_string(),
        "*.min.css".to_string(),
        "*.map".to_string(),
        "dist/".to_string(),
        "build/".to_string(),
        "target/".to_string(),
        "node_modules/".to_string(),
        "*.pb.go".to_string(),
        "*_pb2.py".to_string(),
        "*_pb2_grpc.py".to_string(),
        "*.generated.*".to_string(),
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
            },
            github: None,
            gitlab: None,
            llm: LlmConfig {
                provider: "llamacpp".to_string(),
                base_url: "http://localhost:8080".to_string(),
                model: "default".to_string(),
                max_tokens: 4096,
                temperature: 0.1,
            },
            review: Some(ReviewConfig::default()),
            diff_filter: Some(DiffFilterConfig::default()),
            batching: Some(BatchingConfig::default()),
            database: Some(DatabaseConfig::default()),
            dashboard: Some(DashboardConfig::default()),
            auto_approve: Some(AutoApproveConfig::default()),
            retry: Some(RetryConfig::default()),
            queue: Some(QueueConfig::default()),
            cache: Some(CacheConfig::default()),
            rules: Some(RulesConfig::default()),
            logging: Some(LoggingConfig::default()),
        }
    }
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            security: true,
            style: true,
            performance: true,
            correctness: true,
            maintainability: true,
            inline_comments: true,
            summary_comment: true,
            template: None,
        }
    }
}

impl Default for DiffFilterConfig {
    fn default() -> Self {
        Self {
            lockfile_patterns: default_lockfile_patterns(),
            generated_patterns: default_generated_patterns(),
            enabled: true,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
        }
    }
}

impl Default for BatchingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_seconds: 30,
            max_size: 10,
        }
    }
}

impl Default for AutoApproveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            docs_patterns: default_docs_patterns(),
            skip_lockfiles: true,
            skip_whitespace: true,
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 1000,
            max_delay_ms: 30000,
        }
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            worker_count: 4,
            concurrency_limit: 1,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_hours: 24,
        }
    }
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            rules_dir: default_rules_dir(),
            inline_rules: Vec::new(),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            refresh_seconds: default_dashboard_refresh(),
        }
    }
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let config_path = std::env::var("CONFIG_PATH")
            .unwrap_or_else(|_| "config.toml".to_string());

        let config = if Path::new(&config_path).exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let mut config: AppConfig = toml::from_str(&content)?;

            if config.review.is_none() {
                config.review = Some(ReviewConfig::default());
            }
            if config.diff_filter.is_none() {
                config.diff_filter = Some(DiffFilterConfig::default());
            }
            if config.batching.is_none() {
                config.batching = Some(BatchingConfig::default());
            }
            if config.database.is_none() {
                config.database = Some(DatabaseConfig::default());
            }
            if config.dashboard.is_none() {
                config.dashboard = Some(DashboardConfig::default());
            }
            if config.auto_approve.is_none() {
                config.auto_approve = Some(AutoApproveConfig::default());
            }
            if config.retry.is_none() {
                config.retry = Some(RetryConfig::default());
            }
            if config.queue.is_none() {
                config.queue = Some(QueueConfig::default());
            }
            if config.cache.is_none() {
                config.cache = Some(CacheConfig::default());
            }
            if config.rules.is_none() {
                config.rules = Some(RulesConfig::default());
            }
            if config.logging.is_none() {
                config.logging = Some(LoggingConfig::default());
            }

            config
        } else {
            AppConfig::default()
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration and return descriptive errors.
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut errors = Vec::new();

        // Validate server config
        if self.server.port == 0 {
            errors.push("server.port must be non-zero".to_string());
        }

        // Validate LLM config
        if self.llm.base_url.is_empty() {
            errors.push("llm.base_url must not be empty".to_string());
        }
        if self.llm.model.is_empty() {
            errors.push("llm.model must not be empty".to_string());
        }
        if self.llm.max_tokens == 0 {
            errors.push("llm.max_tokens must be greater than 0".to_string());
        }
        if !(0.0..=2.0).contains(&self.llm.temperature) {
            errors.push("llm.temperature must be between 0.0 and 2.0".to_string());
        }

        // Validate GitHub config if present
        if let Some(ref github) = self.github {
            if github.webhook_secret.is_empty() {
                errors.push("github.webhook_secret must not be empty".to_string());
            }
            if github.app_id.is_empty() {
                errors.push("github.app_id must not be empty".to_string());
            }
            if github.private_key_path.is_empty() {
                errors.push("github.private_key_path must not be empty".to_string());
            }
            if github.use_app_auth && !std::path::Path::new(&github.private_key_path).exists() {
                errors.push(format!(
                    "github.private_key_path '{}' does not exist",
                    github.private_key_path
                ));
            }
        }

        // Validate GitLab config if present
        if let Some(ref gitlab) = self.gitlab {
            if gitlab.webhook_secret.is_empty() {
                errors.push("gitlab.webhook_secret must not be empty".to_string());
            }
            if gitlab.access_token.is_empty() {
                errors.push("gitlab.access_token must not be empty".to_string());
            }
        }

        // Validate that at least one provider is configured
        if self.github.is_none() && self.gitlab.is_none() {
            errors.push("At least one of github or gitlab must be configured".to_string());
        }

        // Validate batching config
        let batching = self.batching_config();
        if batching.timeout_seconds == 0 {
            errors.push("batching.timeout_seconds must be greater than 0".to_string());
        }
        if batching.max_size == 0 {
            errors.push("batching.max_size must be greater than 0".to_string());
        }

        // Validate retry config
        let retry = self.retry_config();
        if retry.max_delay_ms < retry.base_delay_ms {
            errors.push("retry.max_delay_ms must be >= retry.base_delay_ms".to_string());
        }

        // Validate queue config
        let queue = self.queue_config();
        if queue.worker_count == 0 {
            errors.push("queue.worker_count must be greater than 0".to_string());
        }
        if queue.concurrency_limit == 0 {
            errors.push("queue.concurrency_limit must be greater than 0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Configuration validation failed:\n  - {}",
                errors.join("\n  - ")
            ))
        }
    }

    pub fn review_config(&self) -> &ReviewConfig {
        self.review.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<ReviewConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(ReviewConfig::default)
        })
    }

    pub fn diff_filter_config(&self) -> &DiffFilterConfig {
        self.diff_filter.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<DiffFilterConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(DiffFilterConfig::default)
        })
    }

    pub fn batching_config(&self) -> &BatchingConfig {
        self.batching.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<BatchingConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(BatchingConfig::default)
        })
    }

    pub fn database_config(&self) -> &DatabaseConfig {
        self.database.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<DatabaseConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(DatabaseConfig::default)
        })
    }

    pub fn dashboard_config(&self) -> &DashboardConfig {
        self.dashboard.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<DashboardConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(DashboardConfig::default)
        })
    }

    pub fn auto_approve_config(&self) -> &AutoApproveConfig {
        self.auto_approve.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<AutoApproveConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(AutoApproveConfig::default)
        })
    }

    pub fn retry_config(&self) -> &RetryConfig {
        self.retry.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<RetryConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(RetryConfig::default)
        })
    }

    pub fn queue_config(&self) -> &QueueConfig {
        self.queue.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<QueueConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(QueueConfig::default)
        })
    }

    pub fn cache_config(&self) -> &CacheConfig {
        self.cache.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<CacheConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(CacheConfig::default)
        })
    }

    pub fn rules_config(&self) -> &RulesConfig {
        self.rules.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<RulesConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(RulesConfig::default)
        })
    }

    pub fn logging_config(&self) -> &LoggingConfig {
        self.logging.as_ref().unwrap_or_else(|| {
            static DEFAULT: std::sync::OnceLock<LoggingConfig> = std::sync::OnceLock::new();
            DEFAULT.get_or_init(LoggingConfig::default)
        })
    }
}
