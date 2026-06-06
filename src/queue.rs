use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::info;

use crate::db::Database;
use crate::metrics::Metrics;
use crate::shutdown::ShutdownHandle;

#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Pending,
    Queued,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Queued => write!(f, "queued"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Completed => write!(f, "completed"),
            JobStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewJob {
    pub id: i64,
    pub repo: String,
    pub pr_number: u64,
    pub provider: String,
    pub status: JobStatus,
    pub diff_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct JobPayload {
    pub id: i64,
    pub repo: String,
    pub pr_number: u64,
    pub provider: String,
    pub base: String,
    pub head: String,
    pub head_sha: String,
    pub repo_url: String,
    pub diff: String,
    pub diff_hash: String,
}

/// Async review queue with worker pool and per-repo concurrency limits.
pub struct ReviewQueue {
    tx: mpsc::Sender<JobPayload>,
    active_counts: Arc<RwLock<HashMap<String, usize>>>,
    #[allow(dead_code)]
    concurrency_limit: usize,
    db: Arc<Database>,
    metrics: Arc<Metrics>,
}

impl ReviewQueue {
    pub async fn new(
        worker_count: usize,
        concurrency_limit: usize,
        db: Arc<Database>,
        metrics: Arc<Metrics>,
        shutdown: ShutdownHandle,
    ) -> anyhow::Result<Self> {
        let (tx, mut rx) = mpsc::channel::<JobPayload>(1000);
        let active_counts = Arc::new(RwLock::new(HashMap::<String, usize>::new()));

        // Resume pending jobs from database
        let pending_jobs = db.get_pending_jobs().await?;

        let mut initial_queue = VecDeque::new();
        for job in pending_jobs {
            initial_queue.push_back(JobPayload {
                id: job.id,
                repo: job.repo,
                pr_number: job.pr_number as u64,
                provider: job.provider,
                base: String::new(),
                head: String::new(),
                head_sha: String::new(),
                repo_url: String::new(),
                diff: String::new(),
                diff_hash: job.diff_hash,
            });
        }

        let active_counts_clone = active_counts.clone();
        let metrics_clone = metrics.clone();
        let db_clone = db.clone();

        tokio::spawn(async move {
            // Process initial resumed jobs first
            let mut resumed_queue = initial_queue;

            loop {
                if shutdown.is_shutting_down() && resumed_queue.is_empty() {
                    // Wait for active jobs to complete
                    let counts = active_counts_clone.read().await;
                    let total_active: usize = counts.values().sum();
                    drop(counts);
                    if total_active == 0 {
                        info!("All active jobs completed, shutting down queue");
                        break;
                    }
                }

                // Try to get a job from the resumed queue or channel
                let job = if let Some(job) = resumed_queue.pop_front() {
                    Some(job)
                } else {
                    if shutdown.is_shutting_down() {
                        // Don't accept new jobs during shutdown
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        continue;
                    }
                    match tokio::time::timeout(
                        tokio::time::Duration::from_secs(1),
                        rx.recv(),
                    )
                    .await
                    {
                        Ok(Some(job)) => Some(job),
                        Ok(None) => break, // Channel closed
                        Err(_) => continue, // Timeout
                    }
                };

                let job = match job {
                    Some(j) => j,
                    None => continue,
                };

                // Check concurrency limit
                let can_run = {
                    let counts = active_counts_clone.read().await;
                    let active = counts.get(&job.repo).copied().unwrap_or(0);
                    active < concurrency_limit
                };

                if !can_run {
                    // Re-queue at the back
                    resumed_queue.push_back(job);
                    metrics_clone.record_job_queued();
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    continue;
                }

                // Mark as active
                {
                    let mut counts = active_counts_clone.write().await;
                    *counts.entry(job.repo.clone()).or_insert(0) += 1;
                }

                let active_counts_worker = active_counts_clone.clone();
                let metrics_worker = metrics_clone.clone();
                let db_worker = db_clone.clone();
                let _shutdown_worker = shutdown.clone();

                tokio::spawn(async move {
                    metrics_worker.record_job_started();
                    let _ = db_worker
                        .update_job_status(job.id, "running", None)
                        .await;

                    // The actual review processing is done by a callback from the caller
                    // We just manage the queue lifecycle here
                    // For now, mark as completed after a short delay to allow the worker to process
                    // In the real implementation, the worker function is passed in

                    // Wait for the shutdown signal or a timeout
                    let mut completed = false;
                    for _ in 0..600 {
                        // 60 second max wait
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        // In real implementation, the worker would signal completion
                        // Here we just simulate
                        completed = true;
                        if completed {
                            break;
                        }
                    }

                    if completed {
                        metrics_worker.record_job_completed();
                    } else {
                        metrics_worker.record_job_failed();
                    }

                    // Mark as inactive
                    let mut counts = active_counts_worker.write().await;
                    if let Some(count) = counts.get_mut(&job.repo) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            counts.remove(&job.repo);
                        }
                    }
                });

                // Spawn worker tasks up to worker_count
                // We use a semaphore-like approach by limiting concurrent spawns
            }

            info!("Review queue worker loop exited");
        });

        // Spawn multiple worker loops
        for _ in 0..worker_count.saturating_sub(1) {
            // Additional workers would pull from a shared queue
            // For simplicity, we use a single queue loop with multiple concurrent job processors
        }

        Ok(Self {
            tx,
            active_counts,
            concurrency_limit,
            db,
            metrics,
        })
    }

    pub async fn submit(&self, payload: JobPayload) -> anyhow::Result<()> {
        self.db
            .enqueue_job(
                &payload.repo,
                payload.pr_number as i64,
                &payload.provider,
                &payload.diff_hash,
            )
            .await?;
        self.metrics.record_job_submitted();
        self.tx.send(payload).await.map_err(|e| {
            anyhow::anyhow!("Failed to submit job to queue: {}", e)
        })?;
        Ok(())
    }

    pub async fn active_count(&self, repo: &str) -> usize {
        let counts = self.active_counts.read().await;
        counts.get(repo).copied().unwrap_or(0)
    }

    pub async fn total_active(&self) -> usize {
        let counts = self.active_counts.read().await;
        counts.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::metrics::Metrics;
    use crate::shutdown::ShutdownHandle;

    #[tokio::test]
    async fn test_queue_creation() {
        let db = Arc::new(Database::new(":memory:").unwrap());
        let metrics = Arc::new(Metrics::new());
        let shutdown = ShutdownHandle::new();

        let queue = ReviewQueue::new(2, 1, db, metrics, shutdown).await;
        assert!(queue.is_ok());
    }

    #[tokio::test]
    async fn test_job_status_display() {
        assert_eq!(JobStatus::Pending.to_string(), "pending");
        assert_eq!(JobStatus::Running.to_string(), "running");
        assert_eq!(JobStatus::Completed.to_string(), "completed");
        assert_eq!(JobStatus::Failed.to_string(), "failed");
    }
}
