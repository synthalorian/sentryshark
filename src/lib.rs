pub mod config;
pub mod dashboard;
pub mod db;
pub mod diff_filter;
pub mod github;
pub mod gitlab;
pub mod inline_comments;
pub mod llm;
pub mod metrics;
pub mod rate_limit;
pub mod review;

use std::sync::Arc;
use config::AppConfig;
use db::Database;
use metrics::Metrics;
use rate_limit::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub database: Arc<Database>,
    pub metrics: Arc<Metrics>,
    pub rate_limiter: Arc<RateLimiter>,
}
