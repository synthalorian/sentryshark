use std::time::Duration;
use tracing::{debug, warn};

/// Retry configuration for transient failures.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
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

/// Check if an error is transient and should be retried.
pub fn is_transient_error(err: &anyhow::Error) -> bool {
    let err_str = err.to_string().to_lowercase();
    err_str.contains("timeout")
        || err_str.contains("connection")
        || err_str.contains("rate limit")
        || err_str.contains("too many requests")
        || err_str.contains("5")
        || err_str.contains("internal server error")
        || err_str.contains("bad gateway")
        || err_str.contains("service unavailable")
        || err_str.contains("gateway timeout")
}

/// Retry an async operation with exponential backoff and jitter.
pub async fn retry_with_backoff<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    debug!(
                        "{} succeeded after {} retries",
                        operation_name,
                        attempt
                    );
                }
                return Ok(result);
            }
            Err(e) => {
                if !is_transient_error(&e) || attempt == config.max_retries {
                    return Err(e);
                }

                last_error = Some(e);
                let delay = calculate_backoff(attempt, config.base_delay_ms, config.max_delay_ms);
                warn!(
                    "{} failed (attempt {}/{}), retrying in {:?}: {}",
                    operation_name,
                    attempt + 1,
                    config.max_retries + 1,
                    delay,
                    last_error.as_ref().unwrap()
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("All retry attempts exhausted")))
}

fn calculate_backoff(attempt: u32, base_delay_ms: u64, max_delay_ms: u64) -> Duration {
    let exponential = base_delay_ms.saturating_mul(2_u64.saturating_pow(attempt));
    let capped = exponential.min(max_delay_ms);
    // Add jitter (0-25%)
    let jitter = fastrand::u64(0..=capped / 4);
    Duration::from_millis(capped + jitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_is_transient_error() {
        let timeout_err = anyhow::anyhow!("request timeout");
        assert!(is_transient_error(&timeout_err));

        let rate_limit_err = anyhow::anyhow!("rate limit exceeded");
        assert!(is_transient_error(&rate_limit_err));

        let server_err = anyhow::anyhow!("Internal Server Error");
        assert!(is_transient_error(&server_err));

        let not_found_err = anyhow::anyhow!("Not found");
        assert!(!is_transient_error(&not_found_err));
    }

    #[tokio::test]
    async fn test_retry_success_first_attempt() {
        let config = RetryConfig::default();
        let result: anyhow::Result<i32> = retry_with_backoff(&config, "test", || async {
            Ok(42)
        })
        .await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_eventual_success() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result: anyhow::Result<i32> = retry_with_backoff(&config, "test", move || {
            let counter = counter_clone.clone();
            async move {
                let count = counter.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(anyhow::anyhow!("timeout"))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig {
            max_retries: 2,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };

        let result: anyhow::Result<i32> = retry_with_backoff(&config, "test", || async {
            Err(anyhow::anyhow!("rate limit exceeded"))
        })
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_retry_non_transient() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let result: anyhow::Result<i32> = retry_with_backoff(&config, "test", move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(anyhow::anyhow!("not found"))
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1); // No retries for non-transient
    }

    #[test]
    fn test_calculate_backoff() {
        let d0 = calculate_backoff(0, 1000, 30000);
        assert!(d0 >= Duration::from_millis(1000));
        assert!(d0 <= Duration::from_millis(1250));

        let d1 = calculate_backoff(1, 1000, 30000);
        assert!(d1 >= Duration::from_millis(2000));
        assert!(d1 <= Duration::from_millis(2500));

        let d5 = calculate_backoff(5, 1000, 30000);
        assert!(d5 <= Duration::from_millis(30000 + 7500));
    }
}
