/// Token bucket rate limiter for bandwidth throttling
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Token bucket rate limiter
pub struct RateLimiter {
    /// Maximum tokens in the bucket
    max_tokens: u64,
    /// Current tokens in the bucket
    tokens: u64,
    /// Tokens added per second
    refill_rate: u64,
    /// Last refill time
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter
    ///
    /// # Arguments
    /// * `limit_bytes_per_sec` - Maximum bytes per second (0 = unlimited)
    pub fn new(limit_bytes_per_sec: u32) -> Self {
        let max_tokens = if limit_bytes_per_sec == 0 {
            u64::MAX // Unlimited
        } else {
            limit_bytes_per_sec as u64
        };

        Self {
            max_tokens,
            tokens: max_tokens,
            refill_rate: if limit_bytes_per_sec == 0 { u64::MAX } else { limit_bytes_per_sec as u64 },
            last_refill: Instant::now(),
        }
    }

    /// Create an unlimited rate limiter
    pub fn unlimited() -> Self {
        Self::new(0)
    }

    /// Consume tokens from the bucket
    /// Returns the duration to wait before consuming
    pub fn consume(&mut self, tokens: u64) -> Option<Duration> {
        // Refill tokens based on elapsed time
        self.refill();

        if self.tokens >= tokens {
            self.tokens -= tokens;
            None // Can consume immediately
        } else {
            // Calculate how long to wait
            let needed = tokens - self.tokens;
            if self.refill_rate == u64::MAX {
                None // Unlimited, wait a tiny bit
            } else {
                let wait_ms = (needed * 1000) / self.refill_rate;
                Some(Duration::from_millis(wait_ms))
            }
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);

        if elapsed.as_secs_f64() > 0.0 {
            let refill_amount = (elapsed.as_secs_f64() * self.refill_rate as f64) as u64;
            self.tokens = std::cmp::min(self.tokens + refill_amount, self.max_tokens);
            self.last_refill = now;
        }
    }

    /// Get current tokens
    pub fn tokens(&self) -> u64 {
        self.tokens
    }

    /// Check if rate limited
    pub fn is_limited(&self) -> bool {
        self.tokens < self.max_tokens
    }
}

/// Shared rate limiter for concurrent access
#[derive(Clone)]
pub struct SharedRateLimiter {
    inner: Arc<Mutex<RateLimiter>>,
}

impl SharedRateLimiter {
    /// Create a new shared rate limiter
    pub fn new(limit_bytes_per_sec: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiter::new(limit_bytes_per_sec))),
        }
    }

    /// Create an unlimited rate limiter
    pub fn unlimited() -> Self {
        Self::new(0)
    }

    /// Consume tokens, waiting if necessary
    pub async fn consume_and_wait(&self, bytes: u64) {
        loop {
            let wait_duration = {
                let mut limiter = self.inner.lock().await;
                limiter.consume(bytes)
            };

            match wait_duration {
                Some(duration) => {
                    // Wait and retry
                    tokio::time::sleep(duration).await;
                }
                None => {
                    // Can consume immediately
                    break;
                }
            }
        }
    }

    /// Try to consume tokens without waiting
    pub async fn try_consume(&self, bytes: u64) -> bool {
        let mut limiter = self.inner.lock().await;
        limiter.consume(bytes).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unlimited_limiter() {
        let mut limiter = RateLimiter::unlimited();
        assert!(limiter.consume(1_000_000_000).is_none());
    }

    #[test]
    fn test_limited_limiter() {
        let mut limiter = RateLimiter::new(1000); // 1KB/s
        assert!(limiter.consume(500).is_none()); // Should succeed
        let wait = limiter.consume(600); // Should need to wait
        assert!(wait.is_some());
    }

    #[tokio::test]
    async fn test_shared_limiter() {
        let limiter = SharedRateLimiter::new(10000); // 10KB/s
        assert!(limiter.try_consume(5000).await);
        assert!(!limiter.try_consume(6000).await); // Should fail
    }
}
