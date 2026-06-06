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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffFilterConfig {
    #[serde(default = "default_lockfile_patterns")]
    pub lockfile_patterns: Vec<String>,
    #[serde(default = "default_generated_patterns")]
    pub generated_patterns: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
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

fn default_db_path() -> String {
    "sentryshark.db".to_string()
}

fn default_dashboard_refresh() -> u64 {
    30
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
        }
    }
}

impl Default for DiffFilterConfig {
    fn default() -> Self {
        Self {
            lockfile_patterns: default_lockfile_patterns(),
            generated_patterns: default_generated_patterns(),
            enabled: true,
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

        if Path::new(&config_path).exists() {
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

            Ok(config)
        } else {
            Ok(AppConfig::default())
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
}
