use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::inline_comments::{InlineComment, ReviewVerdict, StructuredReview};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub id: i64,
    pub repo: String,
    pub pr_number: i64,
    pub provider: String,
    pub head_sha: String,
    pub verdict: String,
    pub summary: String,
    pub inline_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewStats {
    pub total_reviews: i64,
    pub approved: i64,
    pub request_changes: i64,
    pub commented: i64,
    pub avg_inline_comments: f64,
    pub critical_count: i64,
    pub warning_count: i64,
    pub info_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewJobRecord {
    pub id: i64,
    pub repo: String,
    pub pr_number: i64,
    pub provider: String,
    pub status: String,
    pub diff_hash: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReviewSearchFilters {
    pub repo: Option<String>,
    pub verdict: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub severity: Option<String>,
    pub limit: i64,
}

pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        let mut conn = Connection::open(db_path)?;
        Self::init_schema(&mut conn)?;
        info!("Database initialized at {}", db_path);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init_schema(conn: &mut Connection) -> SqliteResult<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS reviews (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                pr_number INTEGER NOT NULL,
                provider TEXT NOT NULL,
                head_sha TEXT NOT NULL,
                verdict TEXT NOT NULL,
                summary TEXT NOT NULL,
                inline_count INTEGER NOT NULL DEFAULT 0,
                critical_count INTEGER NOT NULL DEFAULT 0,
                warning_count INTEGER NOT NULL DEFAULT 0,
                info_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS review_jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                pr_number INTEGER NOT NULL,
                provider TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                diff_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                error TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS review_cache (
                diff_hash TEXT PRIMARY KEY,
                verdict TEXT NOT NULL,
                summary TEXT NOT NULL,
                inline_comments TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_reviews_repo ON reviews(repo)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_reviews_created ON reviews(created_at)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_reviews_verdict ON reviews(verdict)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_review_jobs_status ON review_jobs(status)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_review_jobs_repo ON review_jobs(repo)",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_review_cache_created ON review_cache(created_at)",
            [],
        )?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_review(
        &self,
        repo: &str,
        pr_number: i64,
        provider: &str,
        head_sha: &str,
        verdict: &str,
        summary: &str,
        inline_count: i64,
        critical_count: i64,
        warning_count: i64,
        info_count: i64,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let created_at = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO reviews (repo, pr_number, provider, head_sha, verdict, summary, inline_count, critical_count, warning_count, info_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![repo, pr_number, provider, head_sha, verdict, summary, inline_count, critical_count, warning_count, info_count, created_at],
        )?;

        let id = conn.last_insert_rowid();
        info!("Saved review {} for {}/{}", id, repo, pr_number);
        Ok(id)
    }

    pub async fn get_recent_reviews(&self, limit: i64) -> anyhow::Result<Vec<ReviewRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, pr_number, provider, head_sha, verdict, summary, inline_count, created_at
             FROM reviews
             ORDER BY created_at DESC
             LIMIT ?1"
        )?;

        let reviews = stmt.query_map(params![limit], |row| {
            let created_at_str: String = row.get(8)?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(ReviewRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                pr_number: row.get(2)?,
                provider: row.get(3)?,
                head_sha: row.get(4)?,
                verdict: row.get(5)?,
                summary: row.get(6)?,
                inline_count: row.get(7)?,
                created_at,
            })
        })?;

        let mut result = Vec::new();
        for review in reviews {
            result.push(review?);
        }

        Ok(result)
    }

    pub async fn get_stats(&self) -> anyhow::Result<ReviewStats> {
        let conn = self.conn.lock().unwrap();

        let total_reviews: i64 = conn.query_row(
            "SELECT COUNT(*) FROM reviews",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let approved: i64 = conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE verdict = 'Approve'",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let request_changes: i64 = conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE verdict = 'RequestChanges'",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let commented: i64 = conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE verdict = 'Comment'",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let avg_inline: f64 = conn.query_row(
            "SELECT COALESCE(AVG(inline_count), 0.0) FROM reviews",
            [],
            |row| row.get(0),
        ).unwrap_or(0.0);

        let critical_count: i64 = conn.query_row(
            "SELECT COALESCE(SUM(critical_count), 0) FROM reviews",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let warning_count: i64 = conn.query_row(
            "SELECT COALESCE(SUM(warning_count), 0) FROM reviews",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let info_count: i64 = conn.query_row(
            "SELECT COALESCE(SUM(info_count), 0) FROM reviews",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        Ok(ReviewStats {
            total_reviews,
            approved,
            request_changes,
            commented,
            avg_inline_comments: avg_inline,
            critical_count,
            warning_count,
            info_count,
        })
    }

    pub async fn get_reviews_by_repo(&self, repo: &str) -> anyhow::Result<Vec<ReviewRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, pr_number, provider, head_sha, verdict, summary, inline_count, created_at
             FROM reviews
             WHERE repo = ?1
             ORDER BY created_at DESC"
        )?;

        let reviews = stmt.query_map(params![repo], |row| {
            let created_at_str: String = row.get(8)?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(ReviewRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                pr_number: row.get(2)?,
                provider: row.get(3)?,
                head_sha: row.get(4)?,
                verdict: row.get(5)?,
                summary: row.get(6)?,
                inline_count: row.get(7)?,
                created_at,
            })
        })?;

        let mut result = Vec::new();
        for review in reviews {
            result.push(review?);
        }

        Ok(result)
    }

    pub async fn search_reviews(
        &self,
        filters: &ReviewSearchFilters,
    ) -> anyhow::Result<Vec<ReviewRecord>> {
        let conn = self.conn.lock().unwrap();

        let mut conditions = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(repo) = &filters.repo {
            conditions.push("repo = ?".to_string());
            params_vec.push(Box::new(repo.clone()));
        }

        if let Some(verdict) = &filters.verdict {
            conditions.push("verdict = ?".to_string());
            params_vec.push(Box::new(verdict.clone()));
        }

        if let Some(from) = filters.from {
            conditions.push("created_at >= ?".to_string());
            params_vec.push(Box::new(from.to_rfc3339()));
        }

        if let Some(to) = filters.to {
            conditions.push("created_at <= ?".to_string());
            params_vec.push(Box::new(to.to_rfc3339()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, repo, pr_number, provider, head_sha, verdict, summary, inline_count, created_at
             FROM reviews
             {}
             ORDER BY created_at DESC
             LIMIT ?",
            where_clause
        );

        params_vec.push(Box::new(filters.limit));
        let params: Vec<&dyn rusqlite::ToSql> = params_vec
            .iter()
            .map(|p| p.as_ref())
            .collect();

        let mut stmt = conn.prepare(&sql)?;
        let reviews = stmt.query_map(params.as_slice(), |row| {
            let created_at_str: String = row.get(8)?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(ReviewRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                pr_number: row.get(2)?,
                provider: row.get(3)?,
                head_sha: row.get(4)?,
                verdict: row.get(5)?,
                summary: row.get(6)?,
                inline_count: row.get(7)?,
                created_at,
            })
        })?;

        let mut result = Vec::new();
        for review in reviews {
            result.push(review?);
        }

        Ok(result)
    }

    // Review job persistence
    pub async fn enqueue_job(
        &self,
        repo: &str,
        pr_number: i64,
        provider: &str,
        diff_hash: &str,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let created_at = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO review_jobs (repo, pr_number, provider, status, diff_hash, created_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5)",
            params![repo, pr_number, provider, diff_hash, created_at],
        )?;

        Ok(conn.last_insert_rowid())
    }

    pub async fn update_job_status(
        &self,
        job_id: i64,
        status: &str,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        match status {
            "running" => {
                conn.execute(
                    "UPDATE review_jobs SET status = ?1, started_at = ?2 WHERE id = ?3",
                    params![status, now, job_id],
                )?;
            }
            "completed" | "failed" => {
                conn.execute(
                    "UPDATE review_jobs SET status = ?1, completed_at = ?2, error = ?3 WHERE id = ?4",
                    params![status, now, error, job_id],
                )?;
            }
            _ => {
                conn.execute(
                    "UPDATE review_jobs SET status = ?1 WHERE id = ?2",
                    params![status, job_id],
                )?;
            }
        }

        Ok(())
    }

    pub async fn get_pending_jobs(&self) -> anyhow::Result<Vec<ReviewJobRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, pr_number, provider, status, diff_hash, created_at, started_at, completed_at, error
             FROM review_jobs
             WHERE status IN ('pending', 'queued')
             ORDER BY created_at ASC"
        )?;

        let jobs = stmt.query_map([], |row| {
            let created_at_str: String = row.get(6)?;
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let started_at = row.get::<_, Option<String>>(7)?.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            });

            let completed_at = row.get::<_, Option<String>>(8)?.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            });

            Ok(ReviewJobRecord {
                id: row.get(0)?,
                repo: row.get(1)?,
                pr_number: row.get(2)?,
                provider: row.get(3)?,
                status: row.get(4)?,
                diff_hash: row.get(5)?,
                created_at,
                started_at,
                completed_at,
                error: row.get(9)?,
            })
        })?;

        let mut result = Vec::new();
        for job in jobs {
            result.push(job?);
        }

        Ok(result)
    }

    // Review cache
    pub async fn get_cached_review(
        &self,
        diff_hash: &str,
        ttl_hours: u64,
    ) -> anyhow::Result<Option<StructuredReview>> {
        let conn = self.conn.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::hours(ttl_hours as i64)).to_rfc3339();

        let result = conn.query_row(
            "SELECT verdict, summary, inline_comments FROM review_cache
             WHERE diff_hash = ?1 AND created_at > ?2",
            params![diff_hash, cutoff],
            |row| {
                let verdict_str: String = row.get(0)?;
                let summary: String = row.get(1)?;
                let comments_json: String = row.get(2)?;

                let verdict = match verdict_str.as_str() {
                    "Approve" => ReviewVerdict::Approve,
                    "RequestChanges" => ReviewVerdict::RequestChanges,
                    _ => ReviewVerdict::Comment,
                };

                let inline_comments: Vec<InlineComment> =
                    serde_json::from_str(&comments_json).unwrap_or_default();

                Ok(StructuredReview {
                    verdict,
                    summary,
                    inline_comments,
                })
            },
        );

        match result {
            Ok(review) => Ok(Some(review)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn save_cached_review(
        &self,
        diff_hash: &str,
        review: &StructuredReview,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let created_at = Utc::now().to_rfc3339();
        let verdict = format!("{:?}", review.verdict);
        let comments_json = serde_json::to_string(&review.inline_comments)?;

        conn.execute(
            "INSERT OR REPLACE INTO review_cache (diff_hash, verdict, summary, inline_comments, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![diff_hash, verdict, &review.summary, comments_json, created_at],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inline_comments::SeverityLevel;

    #[tokio::test]
    async fn test_database_operations() {
        let db = Database::new(":memory:").unwrap();

        let id = db.save_review(
            "test/repo",
            1,
            "github",
            "abc123",
            "Approve",
            "Looks good!",
            0,
            0,
            0,
            0,
        ).await.unwrap();

        assert_eq!(id, 1);

        let reviews = db.get_recent_reviews(10).await.unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].repo, "test/repo");
        assert_eq!(reviews[0].verdict, "Approve");

        let stats = db.get_stats().await.unwrap();
        assert_eq!(stats.total_reviews, 1);
        assert_eq!(stats.approved, 1);

        db.save_review(
            "test/repo",
            2,
            "github",
            "def456",
            "RequestChanges",
            "Needs work",
            3,
            1,
            1,
            1,
        ).await.unwrap();

        let stats = db.get_stats().await.unwrap();
        assert_eq!(stats.total_reviews, 2);
        assert_eq!(stats.request_changes, 1);
        assert_eq!(stats.avg_inline_comments, 1.5);
    }

    #[tokio::test]
    async fn test_search_reviews() {
        let db = Database::new(":memory:").unwrap();

        db.save_review("owner/repo1", 1, "github", "sha1", "Approve", "Good", 0, 0, 0, 0).await.unwrap();
        db.save_review("owner/repo2", 2, "github", "sha2", "Comment", "OK", 0, 0, 0, 0).await.unwrap();
        db.save_review("owner/repo1", 3, "github", "sha3", "RequestChanges", "Bad", 0, 0, 0, 0).await.unwrap();

        let filters = ReviewSearchFilters {
            repo: Some("owner/repo1".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = db.search_reviews(&filters).await.unwrap();
        assert_eq!(results.len(), 2);

        let filters = ReviewSearchFilters {
            verdict: Some("Approve".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = db.search_reviews(&filters).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].verdict, "Approve");
    }

    #[tokio::test]
    async fn test_job_persistence() {
        let db = Database::new(":memory:").unwrap();

        let id = db.enqueue_job("test/repo", 1, "github", "hash123").await.unwrap();
        assert_eq!(id, 1);

        let pending = db.get_pending_jobs().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].repo, "test/repo");
        assert_eq!(pending[0].status, "pending");

        db.update_job_status(id, "running", None).await.unwrap();
        db.update_job_status(id, "completed", None).await.unwrap();

        let pending = db.get_pending_jobs().await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_review_cache() {
        let db = Database::new(":memory:").unwrap();

        let review = StructuredReview {
            verdict: ReviewVerdict::Approve,
            summary: "Looks good".to_string(),
            inline_comments: vec![InlineComment {
                file_path: "src/main.rs".to_string(),
                line: 42,
                body: "Nice".to_string(),
                severity: SeverityLevel::Info,
            }],
        };

        db.save_cached_review("hash1", &review).await.unwrap();

        let cached = db.get_cached_review("hash1", 24).await.unwrap();
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.verdict, ReviewVerdict::Approve);
        assert_eq!(cached.inline_comments.len(), 1);

        let expired = db.get_cached_review("hash1", 0).await.unwrap();
        assert!(expired.is_none());
    }
}
