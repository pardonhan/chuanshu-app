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

    /// Consume tokens from the bucket
    /// Returns the duration to wait before consuming
    pub fn consume(&mut self, tokens: u64) -> Option<Duration> {
        // 不限速时直接放行，避免无意义的计数与溢出
        if self.refill_rate == u64::MAX {
            return None;
        }

        // Refill tokens based on elapsed time
        self.refill();

        if self.tokens >= tokens {
            self.tokens -= tokens;
            None // Can consume immediately
        } else {
            // Calculate how long to wait
            let needed = tokens - self.tokens;
            let wait_ms = (needed * 1000) / self.refill_rate;
            Some(Duration::from_millis(wait_ms))
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);

        if elapsed.as_secs_f64() > 0.0 {
            let refill_amount = (elapsed.as_secs_f64() * self.refill_rate as f64) as u64;
            self.tokens = self.tokens.saturating_add(refill_amount).min(self.max_tokens);
            self.last_refill = now;
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unlimited_limiter() {
        let mut limiter = RateLimiter::new(0);
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
        let limiter = SharedRateLimiter::new(0);
        limiter.consume_and_wait(1_000_000).await;
    }
}
