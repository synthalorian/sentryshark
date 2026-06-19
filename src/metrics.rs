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
    reviews_auto_approved: AtomicU64,
    review_latency_ms: AtomicU64,
    review_latency_count: AtomicU64,
    webhooks_received: AtomicU64,
    webhooks_rejected: AtomicU64,
    webhooks_rate_limited: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    jobs_submitted: AtomicU64,
    jobs_started: AtomicU64,
    jobs_completed: AtomicU64,
    jobs_failed: AtomicU64,
    jobs_queued: AtomicU64,
    critical_findings: AtomicU64,
    warning_findings: AtomicU64,
    info_findings: AtomicU64,
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
            reviews_auto_approved: AtomicU64::new(0),
            review_latency_ms: AtomicU64::new(0),
            review_latency_count: AtomicU64::new(0),
            webhooks_received: AtomicU64::new(0),
            webhooks_rejected: AtomicU64::new(0),
            webhooks_rate_limited: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            jobs_submitted: AtomicU64::new(0),
            jobs_started: AtomicU64::new(0),
            jobs_completed: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            jobs_queued: AtomicU64::new(0),
            critical_findings: AtomicU64::new(0),
            warning_findings: AtomicU64::new(0),
            info_findings: AtomicU64::new(0),
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

        if let Ok(mut stats) = self.per_repo_stats.lock() {
            let entry = stats.entry(repo.to_string()).or_default();
            entry.reviews += 1;
            match verdict {
                "Approve" => entry.approved += 1,
                "RequestChanges" => entry.request_changes += 1,
                _ => entry.commented += 1,
            }
        }
    }

    pub fn record_review_failed(&self) {
        self.reviews_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_auto_approve(&self) {
        self.reviews_auto_approved.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_job_submitted(&self) {
        self.jobs_submitted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_job_started(&self) {
        self.jobs_started.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_job_completed(&self) {
        self.jobs_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_job_failed(&self) {
        self.jobs_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_job_queued(&self) {
        self.jobs_queued.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_severity_counts(&self, critical: u64, warning: u64, info: u64) {
        self.critical_findings.fetch_add(critical, Ordering::Relaxed);
        self.warning_findings.fetch_add(warning, Ordering::Relaxed);
        self.info_findings.fetch_add(info, Ordering::Relaxed);
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

        output.push_str("# HELP sentryshark_reviews_auto_approved Total number of auto-approved reviews.\n");
        output.push_str("# TYPE sentryshark_reviews_auto_approved counter\n");
        output.push_str(&format!(
            "sentryshark_reviews_auto_approved {}\n",
            self.reviews_auto_approved.load(Ordering::Relaxed)
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

        output.push_str("# HELP sentryshark_cache_hits Total cache hits.\n");
        output.push_str("# TYPE sentryshark_cache_hits counter\n");
        output.push_str(&format!(
            "sentryshark_cache_hits {}\n",
            self.cache_hits.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_cache_misses Total cache misses.\n");
        output.push_str("# TYPE sentryshark_cache_misses counter\n");
        output.push_str(&format!(
            "sentryshark_cache_misses {}\n",
            self.cache_misses.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_jobs_submitted Total jobs submitted.\n");
        output.push_str("# TYPE sentryshark_jobs_submitted counter\n");
        output.push_str(&format!(
            "sentryshark_jobs_submitted {}\n",
            self.jobs_submitted.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_jobs_started Total jobs started.\n");
        output.push_str("# TYPE sentryshark_jobs_started counter\n");
        output.push_str(&format!(
            "sentryshark_jobs_started {}\n",
            self.jobs_started.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_jobs_completed Total jobs completed.\n");
        output.push_str("# TYPE sentryshark_jobs_completed counter\n");
        output.push_str(&format!(
            "sentryshark_jobs_completed {}\n",
            self.jobs_completed.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_jobs_failed Total jobs failed.\n");
        output.push_str("# TYPE sentryshark_jobs_failed counter\n");
        output.push_str(&format!(
            "sentryshark_jobs_failed {}\n",
            self.jobs_failed.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_critical_findings Total critical severity findings.\n");
        output.push_str("# TYPE sentryshark_critical_findings counter\n");
        output.push_str(&format!(
            "sentryshark_critical_findings {}\n",
            self.critical_findings.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_warning_findings Total warning severity findings.\n");
        output.push_str("# TYPE sentryshark_warning_findings counter\n");
        output.push_str(&format!(
            "sentryshark_warning_findings {}\n",
            self.warning_findings.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP sentryshark_info_findings Total info severity findings.\n");
        output.push_str("# TYPE sentryshark_info_findings counter\n");
        output.push_str(&format!(
            "sentryshark_info_findings {}\n",
            self.info_findings.load(Ordering::Relaxed)
        ));

        if let Ok(repo_stats) = self.per_repo_stats.lock() {
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
        }

        output
    }

    /// Render a simple JSON metrics response.
    pub fn render_json(&self) -> serde_json::Value {
        let mut repos = serde_json::Map::new();
        if let Ok(repo_stats) = self.per_repo_stats.lock() {
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
        }

        serde_json::json!({
            "reviews_total": self.reviews_total.load(Ordering::Relaxed),
            "reviews_approved": self.reviews_approved.load(Ordering::Relaxed),
            "reviews_request_changes": self.reviews_request_changes.load(Ordering::Relaxed),
            "reviews_commented": self.reviews_commented.load(Ordering::Relaxed),
            "reviews_failed": self.reviews_failed.load(Ordering::Relaxed),
            "reviews_auto_approved": self.reviews_auto_approved.load(Ordering::Relaxed),
            "avg_latency_ms": self.avg_latency_ms(),
            "webhooks_received": self.webhooks_received.load(Ordering::Relaxed),
            "webhooks_rejected": self.webhooks_rejected.load(Ordering::Relaxed),
            "webhooks_rate_limited": self.webhooks_rate_limited.load(Ordering::Relaxed),
            "cache_hits": self.cache_hits.load(Ordering::Relaxed),
            "cache_misses": self.cache_misses.load(Ordering::Relaxed),
            "jobs_submitted": self.jobs_submitted.load(Ordering::Relaxed),
            "jobs_started": self.jobs_started.load(Ordering::Relaxed),
            "jobs_completed": self.jobs_completed.load(Ordering::Relaxed),
            "jobs_failed": self.jobs_failed.load(Ordering::Relaxed),
            "critical_findings": self.critical_findings.load(Ordering::Relaxed),
            "warning_findings": self.warning_findings.load(Ordering::Relaxed),
            "info_findings": self.info_findings.load(Ordering::Relaxed),
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
        metrics.record_auto_approve();
        metrics.record_cache_hit();
        metrics.record_cache_miss();
        metrics.record_job_submitted();
        metrics.record_job_started();
        metrics.record_job_completed();
        metrics.record_job_failed();
        metrics.record_job_queued();
        metrics.record_severity_counts(1, 2, 3);

        assert_eq!(metrics.webhooks_received.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.webhooks_rejected.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_total.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.reviews_approved.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_request_changes.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_failed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.reviews_auto_approved.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.cache_hits.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.cache_misses.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.jobs_submitted.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.jobs_started.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.jobs_completed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.jobs_failed.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.critical_findings.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.warning_findings.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.info_findings.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn test_prometheus_format() {
        let metrics = Metrics::new();
        metrics.record_review("Approve", "test/repo", None);

        let output = metrics.render_prometheus().await;
        assert!(output.contains("sentryshark_reviews_total 1"));
        assert!(output.contains("sentryshark_reviews_approved 1"));
        assert!(output.contains("# TYPE sentryshark_reviews_total counter"));
        assert!(output.contains("sentryshark_cache_hits"));
        assert!(output.contains("sentryshark_critical_findings"));
    }

    #[test]
    fn test_metrics_json() {
        let metrics = Metrics::new();
        metrics.record_review("Comment", "test/repo", None);

        let json = metrics.render_json();
        assert_eq!(json["reviews_total"], 1);
        assert_eq!(json["reviews_commented"], 1);
        assert!(json["repos"]["test/repo"].is_object());
        assert!(json["cache_hits"].is_number());
    }
}
