use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Prometheus-compatible metrics for SentryShark.
#[derive(Debug)]
pub struct Metrics {
    reviews_total: AtomicU64,
    reviews_approved: AtomicU64,
    reviews_request_changes: AtomicU64,
    reviews_commented: AtomicU64,
    reviews_failed: AtomicU64,
    review_latency_ms: AtomicU64,
    review_latency_count: AtomicU64,
    webhooks_received: AtomicU64,
    webhooks_rejected: AtomicU64,
    webhooks_rate_limited: AtomicU64,
    per_repo_stats: Arc<Mutex<std::collections::HashMap<String, RepoMetrics>>>,
}

#[derive(Debug, Default)]
pub struct RepoMetrics {
    pub reviews: u64,
    pub approved: u64,
    pub request_changes: u64,
    pub commented: u64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            reviews_total: AtomicU64::new(0),
            reviews_approved: AtomicU64::new(0),
            reviews_request_changes: AtomicU64::new(0),
            reviews_commented: AtomicU64::new(0),
            reviews_failed: AtomicU64::new(0),
            review_latency_ms: AtomicU64::new(0),
            review_latency_count: AtomicU64::new(0),
            webhooks_received: AtomicU64::new(0),
            webhooks_rejected: AtomicU64::new(0),
            webhooks_rate_limited: AtomicU64::new(0),
            per_repo_stats: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn record_webhook_received(&self) {
        self.webhooks_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_webhook_rejected(&self) {
        self.webhooks_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_webhook_rate_limited(&self) {
        self.webhooks_rate_limited.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_review(&self, verdict: &str, repo: &str, latency: Option<Instant>) {
        self.reviews_total.fetch_add(1, Ordering::Relaxed);

        match verdict {
            "Approve" => self.reviews_approved.fetch_add(1, Ordering::Relaxed),
            "RequestChanges" => self.reviews_request_changes.fetch_add(1, Ordering::Relaxed),
            _ => self.reviews_commented.fetch_add(1, Ordering::Relaxed),
        };

        if let Some(start) = latency {
            let elapsed = start.elapsed().as_millis() as u64;
            self.review_latency_ms.fetch_add(elapsed, Ordering::Relaxed);
            self.review_latency_count.fetch_add(1, Ordering::Relaxed);
        }

        let mut stats = self.per_repo_stats.lock().unwrap();
        let entry = stats.entry(repo.to_string()).or_default();
        entry.reviews += 1;
        match verdict {
            "Approve" => entry.approved += 1,
            "RequestChanges" => entry.request_changes += 1,
            _ => entry.commented += 1,
        }
    }

    pub fn record_review_failed(&self) {
        self.reviews_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn avg_latency_ms(&self) -> f64 {
        let total = self.review_latency_ms.load(Ordering::Relaxed);
        let count = self.review_latency_count.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            total as f64 / count as f64
        }
    }

    /// Render metrics in Prometheus exposition format.
    pub async fn render_prometheus(&self) -> String {
        let mut output = String::new();

        output.push_str("# HELP sentryshark_reviews_total Total number of code reviews performed.\n");
        output.push_str("# TYPE sentryshark_reviews_total counter\n");
        output.push_str(&format!(
            "sentryshark_reviews_total {}\n",
            self.reviews_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_reviews_approved Total number of approved reviews.\n");
        output.push_str("# TYPE sentryshark_reviews_approved counter\n");
        output.push_str(&format!(
            "sentryshark_reviews_approved {}\n",
            self.reviews_approved.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_reviews_request_changes Total number of reviews requesting changes.\n");
        output.push_str("# TYPE sentryshark_reviews_request_changes counter\n");
        output.push_str(&format!(
            "sentryshark_reviews_request_changes {}\n",
            self.reviews_request_changes.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_reviews_commented Total number of comment-only reviews.\n");
        output.push_str("# TYPE sentryshark_reviews_commented counter\n");
        output.push_str(&format!(
            "sentryshark_reviews_commented {}\n",
            self.reviews_commented.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_reviews_failed Total number of failed reviews.\n");
        output.push_str("# TYPE sentryshark_reviews_failed counter\n");
        output.push_str(&format!(
            "sentryshark_reviews_failed {}\n",
            self.reviews_failed.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_review_latency_ms Average review latency in milliseconds.\n");
        output.push_str("# TYPE sentryshark_review_latency_ms gauge\n");
        output.push_str(&format!(
            "sentryshark_review_latency_ms {:.2}\n",
            self.avg_latency_ms()
        ));

        output.push_str("# HELP sentryshark_webhooks_received Total webhooks received.\n");
        output.push_str("# TYPE sentryshark_webhooks_received counter\n");
        output.push_str(&format!(
            "sentryshark_webhooks_received {}\n",
            self.webhooks_received.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_webhooks_rejected Total webhooks rejected (auth failure).\n");
        output.push_str("# TYPE sentryshark_webhooks_rejected counter\n");
        output.push_str(&format!(
            "sentryshark_webhooks_rejected {}\n",
            self.webhooks_rejected.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_webhooks_rate_limited Total webhooks rate limited.\n");
        output.push_str("# TYPE sentryshark_webhooks_rate_limited counter\n");
        output.push_str(&format!(
            "sentryshark_webhooks_rate_limited {}\n",
            self.webhooks_rate_limited.load(Ordering::Relaxed)
        ));

        let repo_stats = self.per_repo_stats.lock().unwrap();
        if !repo_stats.is_empty() {
            output.push_str("# HELP sentryshark_repo_reviews Total reviews per repository.\n");
            output.push_str("# TYPE sentryshark_repo_reviews counter\n");
            for (repo, stats) in repo_stats.iter() {
                let repo_escaped = repo.replace('\\', "\\\\").replace('"', "\\\"");
                output.push_str(&format!(
                    "sentryshark_repo_reviews{{repo=\"{}\"}} {}\n",
                    repo_escaped, stats.reviews
                ));
            }
        }

        output
    }

    /// Render a simple JSON metrics response.
    pub fn render_json(&self) -> serde_json::Value {
        let repo_stats = self.per_repo_stats.lock().unwrap();
        let mut repos = serde_json::Map::new();
        for (repo, stats) in repo_stats.iter() {
            repos.insert(
                repo.clone(),
                serde_json::json!({
                    "reviews": stats.reviews,
                    "approved": stats.approved,
                    "request_changes": stats.request_changes,
                    "commented": stats.commented,
                }),
            );
        }

        serde_json::json!({
            "reviews_total": self.reviews_total.load(Ordering::Relaxed),
            "reviews_approved": self.reviews_approved.load(Ordering::Relaxed),
            "reviews_request_changes": self.reviews_request_changes.load(Ordering::Relaxed),
            "reviews_commented": self.reviews_commented.load(Ordering::Relaxed),
            "reviews_failed": self.reviews_failed.load(Ordering::Relaxed),
            "avg_latency_ms": self.avg_latency_ms(),
            "webhooks_received": self.webhooks_received.load(Ordering::Relaxed),
            "webhooks_rejected": self.webhooks_rejected.load(Ordering::Relaxed),
            "webhooks_rate_limited": self.webhooks_rate_limited.load(Ordering::Relaxed),
            "repos": repos,
        })
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_counters() {
        let metrics = Metrics::new();
        metrics.record_webhook_received();
        metrics.record_webhook_received();
        metrics.record_webhook_rejected();
        metrics.record_review("Approve", "test/repo", None);
        metrics.record_review("RequestChanges", "test/repo", None);
        metrics.record_review_failed();

        assert_eq!(metrics.webhooks_received.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.webhooks_rejected.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_total.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.reviews_approved.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_request_changes.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_failed.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_prometheus_format() {
        let metrics = Metrics::new();
        metrics.record_review("Approve", "test/repo", None);

        let output = metrics.render_prometheus().await;
        assert!(output.contains("sentryshark_reviews_total 1"));
        assert!(output.contains("sentryshark_reviews_approved 1"));
        assert!(output.contains("# TYPE sentryshark_reviews_total counter"));
    }

    #[test]
    fn test_metrics_json() {
        let metrics = Metrics::new();
        metrics.record_review("Comment", "test/repo", None);

        let json = metrics.render_json();
        assert_eq!(json["reviews_total"], 1);
        assert_eq!(json["reviews_commented"], 1);
        assert!(json["repos"]["test/repo"].is_object());
    }
}
