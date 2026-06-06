use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Simple in-memory rate limiter for webhook endpoints.
/// Limits requests per IP address within a time window.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    max_requests: u32,
    window: Duration,
    store: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_seconds),
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if a request from the given key is allowed.
    /// Returns true if the request is allowed, false if rate limited.
    pub async fn is_allowed(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut store = self.store.lock().await;

        let timestamps = store.entry(key.to_string()).or_default();

        // Remove timestamps outside the window
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests as usize {
            false
        } else {
            timestamps.push(now);
            true
        }
    }

    /// Get current request count for a key.
    pub async fn count(&self, key: &str) -> usize {
        let now = Instant::now();
        let mut store = self.store.lock().await;

        if let Some(timestamps) = store.get_mut(key) {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            timestamps.len()
        } else {
            0
        }
    }

    /// Clean up expired entries to prevent unbounded growth.
    pub async fn cleanup(&self) {
        let now = Instant::now();
        let mut store = self.store.lock().await;
        store.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            !timestamps.is_empty()
        });
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(60, 60) // 60 requests per minute
    }
}

/// Extract client IP from request info.
pub fn extract_client_key(addr: Option<SocketAddr>) -> String {
    addr.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new(3, 60);

        assert!(limiter.is_allowed("client1").await);
        assert!(limiter.is_allowed("client1").await);
        assert!(limiter.is_allowed("client1").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_blocks_over_limit() {
        let limiter = RateLimiter::new(2, 60);

        assert!(limiter.is_allowed("client1").await);
        assert!(limiter.is_allowed("client1").await);
        assert!(!limiter.is_allowed("client1").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_independent_keys() {
        let limiter = RateLimiter::new(1, 60);

        assert!(limiter.is_allowed("client1").await);
        assert!(limiter.is_allowed("client2").await);
        assert!(!limiter.is_allowed("client1").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_window_expires() {
        let limiter = RateLimiter::new(1, 1);

        assert!(limiter.is_allowed("client1").await);
        assert!(!limiter.is_allowed("client1").await);

        // Wait for window to expire
        sleep(Duration::from_secs(2)).await;
        assert!(limiter.is_allowed("client1").await);
    }

    #[tokio::test]
    async fn test_cleanup() {
        let limiter = RateLimiter::new(10, 1);
        limiter.is_allowed("client1").await;

        sleep(Duration::from_secs(2)).await;
        limiter.cleanup().await;

        assert_eq!(limiter.count("client1").await, 0);
    }
}
