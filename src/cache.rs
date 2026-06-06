use crate::db::Database;
use crate::inline_comments::StructuredReview;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::{debug, info};

/// Cache for review results to avoid re-reviewing identical diffs.
pub struct ReviewCache {
    db: Arc<Database>,
    ttl_hours: u64,
}

impl ReviewCache {
    pub fn new(db: Arc<Database>, ttl_hours: u64) -> Self {
        Self { db, ttl_hours }
    }

    pub fn compute_key(diff: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(diff.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub async fn get(&self, diff: &str) -> anyhow::Result<Option<StructuredReview>> {
        let key = Self::compute_key(diff);
        debug!("Checking cache for diff hash: {}", &key[..16]);

        match self.db.get_cached_review(&key, self.ttl_hours).await? {
            Some(cached) => {
                info!("Cache hit for diff hash: {}", &key[..16]);
                Ok(Some(cached))
            }
            None => {
                debug!("Cache miss for diff hash: {}", &key[..16]);
                Ok(None)
            }
        }
    }

    pub async fn set(&self, diff: &str, review: &StructuredReview) -> anyhow::Result<()> {
        let key = Self::compute_key(diff);
        self.db.save_cached_review(&key, review).await?;
        debug!("Cached review result for diff hash: {}", &key[..16]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::inline_comments::{InlineComment, ReviewVerdict, StructuredReview};

    #[tokio::test]
    async fn test_cache_hit_miss() {
        let db = Arc::new(Database::new(":memory:").unwrap());
        let cache = ReviewCache::new(db.clone(), 24);

        let diff = "some diff content";
        let key = ReviewCache::compute_key(diff);
        assert_eq!(key.len(), 64);

        // Initially miss
        let result = cache.get(diff).await.unwrap();
        assert!(result.is_none());

        // Store a review
        let review = StructuredReview {
            verdict: ReviewVerdict::Approve,
            summary: "Looks good".to_string(),
            inline_comments: vec![InlineComment {
                file_path: "src/main.rs".to_string(),
                line: 42,
                body: "Nice".to_string(),
                severity: crate::inline_comments::SeverityLevel::Info,
            }],
        };

        cache.set(diff, &review).await.unwrap();

        // Now hit
        let result = cache.get(diff).await.unwrap();
        assert!(result.is_some());
        let cached = result.unwrap();
        assert_eq!(cached.verdict, ReviewVerdict::Approve);
        assert_eq!(cached.inline_comments.len(), 1);
    }

    #[tokio::test]
    async fn test_cache_different_diffs() {
        let db = Arc::new(Database::new(":memory:").unwrap());
        let cache = ReviewCache::new(db.clone(), 24);

        let diff1 = "diff content 1";
        let diff2 = "diff content 2";

        let review = StructuredReview {
            verdict: ReviewVerdict::Approve,
            summary: "Good".to_string(),
            inline_comments: vec![],
        };

        cache.set(diff1, &review).await.unwrap();

        assert!(cache.get(diff1).await.unwrap().is_some());
        assert!(cache.get(diff2).await.unwrap().is_none());
    }

    #[test]
    fn test_compute_key_deterministic() {
        let diff = "test diff";
        let key1 = ReviewCache::compute_key(diff);
        let key2 = ReviewCache::compute_key(diff);
        assert_eq!(key1, key2);
    }
}
