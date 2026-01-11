//! Rate Limiting for MaDRPC
//!
//! This module provides rate limiting support for MaDRPC nodes and orchestrators.
//! The current implementation uses a token bucket algorithm with per-IP rate limits.
//!
//! # Architecture
//!
//! Rate limiting is optional and can be configured per server:
//! - **No rate limiting**: Server accepts all requests (default for backward compatibility)
//! - **Per-IP rate limiting**: Server limits requests per IP address using token bucket
//!
//! # Security Model
//!
//! - Token bucket algorithm allows bursts while limiting sustained request rates
//! - Per-IP tracking prevents malicious clients from overwhelming the server
//! - Automatic cleanup of stale entries prevents memory exhaustion
//! - Rate limit exceeded results in HTTP 429 Too Many Requests
//!
//! # Example
//!
//! ```no_run
//! use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
//!
//! // Create a rate limiter with 10 requests per second per IP
//! let config = RateLimitConfig::per_second(10);
//! let limiter = RateLimiter::new(config);
//!
//! // Check if a request should be allowed
//! if limiter.check_rate_limit("127.0.0.1").is_allowed() {
//!     // Process the request
//! } else {
//!     // Return HTTP 429
//! }
//! ```

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Configuration for rate limiting.
///
/// Defines how aggressively to limit requests from individual IP addresses.
/// Uses a token bucket algorithm that allows bursts while limiting sustained rates.
///
/// # Fields
///
/// * `requests_per_second` - Maximum sustained request rate
/// * `burst_size` - Maximum number of requests allowed in a burst
/// * `cleanup_interval` - How often to clean up stale IP entries
/// * `entry_ttl` - How long to keep IP entries without activity
///
/// # Example
///
/// ```
/// use madrpc_common::rate_limit::RateLimitConfig;
///
/// // 10 requests per second, burst of 20
/// let config = RateLimitConfig::new(10.0, 20);
/// assert_eq!(config.requests_per_second, 10.0);
/// assert_eq!(config.burst_size, 20);
/// ```
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum sustained request rate (requests per second)
    pub requests_per_second: f64,
    /// Maximum burst size (number of tokens)
    pub burst_size: u32,
    /// Interval for cleaning up stale entries
    pub cleanup_interval: Duration,
    /// Time-to-live for entries without activity
    pub entry_ttl: Duration,
}

impl RateLimitConfig {
    /// Creates a new rate limit configuration.
    ///
    /// # Arguments
    ///
    /// * `requests_per_second` - Maximum sustained request rate
    /// * `burst_size` - Maximum burst size (should be >= requests_per_second for reasonable bursts)
    ///
    /// # Returns
    ///
    /// A new `RateLimitConfig` with default cleanup settings
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::RateLimitConfig;
    ///
    /// let config = RateLimitConfig::new(10.0, 20);
    /// ```
    pub fn new(requests_per_second: f64, burst_size: u32) -> Self {
        Self {
            requests_per_second,
            burst_size,
            cleanup_interval: Duration::from_secs(60),
            entry_ttl: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Creates a configuration for requests per second limiting.
    ///
    /// Sets burst size to 2x the rate to allow reasonable bursts.
    ///
    /// # Arguments
    ///
    /// * `rps` - Requests per second
    ///
    /// # Returns
    ///
    /// A new `RateLimitConfig` with burst size = 2 * rps
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::RateLimitConfig;
    ///
    /// let config = RateLimitConfig::per_second(10);
    /// assert_eq!(config.requests_per_second, 10.0);
    /// assert_eq!(config.burst_size, 20);
    /// ```
    pub fn per_second(rps: f64) -> Self {
        let burst_size = (rps * 2.0).ceil() as u32;
        Self::new(rps, burst_size)
    }

    /// Creates a configuration for requests per minute limiting.
    ///
    /// # Arguments
    ///
    /// * `rpm` - Requests per minute
    ///
    /// # Returns
    ///
    /// A new `RateLimitConfig` converted to per-second rate
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::RateLimitConfig;
    ///
    /// let config = RateLimitConfig::per_minute(600);
    /// assert_eq!(config.requests_per_second, 10.0);
    /// ```
    pub fn per_minute(rpm: u32) -> Self {
        let rps = rpm as f64 / 60.0;
        Self::per_second(rps)
    }
}

impl Default for RateLimitConfig {
    /// Creates a default configuration with rate limiting disabled.
    fn default() -> Self {
        Self {
            requests_per_second: f64::MAX,
            burst_size: u32::MAX,
            cleanup_interval: Duration::from_secs(60),
            entry_ttl: Duration::from_secs(300),
        }
    }
}

/// Result of a rate limit check.
///
/// Indicates whether a request should be allowed or rate limited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateLimitResult {
    /// Request is allowed
    Allowed,
    /// Request is rate limited
    RateLimited {
        /// Time until the next request will be allowed
        retry_after: Duration,
    },
}

impl RateLimitResult {
    /// Returns whether the request is allowed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// Returns the retry-after duration if rate limited.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Allowed => None,
            Self::RateLimited { retry_after } => Some(*retry_after),
        }
    }
}

/// Token bucket state for a single IP address.
///
/// Tracks the number of available tokens and the last update time.
#[derive(Debug)]
struct TokenBucket {
    /// Current number of available tokens
    tokens: f64,
    /// Last time this bucket was updated
    last_update: Instant,
}

impl TokenBucket {
    /// Creates a new token bucket with full tokens.
    fn new(burst_size: u32) -> Self {
        Self {
            tokens: burst_size as f64,
            last_update: Instant::now(),
        }
    }

    /// Attempts to consume a token from the bucket.
    ///
    /// # Arguments
    ///
    /// * `config` - Rate limit configuration
    /// * `now` - Current time
    ///
    /// # Returns
    ///
    /// `true` if a token was consumed, `false` if rate limited
    fn try_consume(&mut self, config: &RateLimitConfig, now: Instant) -> bool {
        // Calculate time elapsed since last update
        let elapsed = now.duration_since(self.last_update);
        let elapsed_secs = elapsed.as_secs_f64();

        // Add tokens based on elapsed time (token refill)
        let new_tokens = elapsed_secs * config.requests_per_second;
        self.tokens = (self.tokens + new_tokens).min(config.burst_size as f64);
        self.last_update = now;

        // Try to consume a token
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Returns the time until the next token will be available.
    ///
    /// # Arguments
    ///
    /// * `config` - Rate limit configuration
    ///
    /// # Returns
    ///
    /// Duration until next token is available
    fn time_until_next_token(&self, config: &RateLimitConfig) -> Duration {
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let tokens_needed = 1.0 - self.tokens;
            let secs_needed = tokens_needed / config.requests_per_second;
            Duration::from_secs_f64(secs_needed)
        }
    }
}

/// Rate limiter using token bucket algorithm.
///
/// Tracks request rates per IP address and enforces configured limits.
/// Automatically cleans up stale entries to prevent memory leaks.
///
/// # Thread Safety
///
/// This type uses an Arc<RwLock<HashMap>> internally and is safe to share
/// across threads. Cloning is cheap as it creates a new handle to the same
/// underlying data.
///
/// # Example
///
/// ```
/// use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
/// use std::net::IpAddr;
/// use std::str::FromStr;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let config = RateLimitConfig::per_second(10);
/// let limiter = RateLimiter::new(config);
///
/// let ip: IpAddr = FromStr::from_str("127.0.0.1").unwrap();
///
/// // First request is allowed
/// assert!(limiter.check_rate_limit(&ip).await.is_allowed());
///
/// // Exhaust the rate limit
/// for _ in 0..20 {
///     limiter.check_rate_limit(&ip).await;
/// }
///
/// // Next request is rate limited
/// assert!(!limiter.check_rate_limit(&ip).await.is_allowed());
/// # });
/// ```
#[derive(Clone)]
pub struct RateLimiter {
    /// Rate limit configuration
    pub config: RateLimitConfig,
    /// Per-IP token buckets
    buckets: Arc<RwLock<HashMap<IpAddr, TokenBucket>>>,
    /// Last cleanup time
    last_cleanup: Arc<RwLock<Instant>>,
}

impl RateLimiter {
    /// Creates a new rate limiter with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Rate limit configuration
    ///
    /// # Returns
    ///
    /// A new `RateLimiter` instance
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
    ///
    /// let config = RateLimitConfig::per_second(10);
    /// let limiter = RateLimiter::new(config);
    /// ```
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Arc::new(RwLock::new(HashMap::new())),
            last_cleanup: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Creates a rate limiter with rate limiting disabled.
    ///
    /// This is the default for backward compatibility.
    ///
    /// # Returns
    ///
    /// A new `RateLimiter` instance with unlimited rate
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::RateLimiter;
    ///
    /// let limiter = RateLimiter::disabled();
    /// // All requests will be allowed
    /// ```
    pub fn disabled() -> Self {
        Self::new(RateLimitConfig::default())
    }

    /// Checks if a request from the given IP should be rate limited.
    ///
    /// This method:
    /// 1. Periodically cleans up stale entries (based on cleanup_interval)
    /// 2. Gets or creates a token bucket for the IP
    /// 3. Attempts to consume a token
    /// 4. Returns whether the request is allowed
    ///
    /// # Arguments
    ///
    /// * `ip` - The IP address of the client
    ///
    /// # Returns
    ///
    /// A `RateLimitResult` indicating if the request is allowed or rate limited
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
    /// use std::net::IpAddr;
    /// use std::str::FromStr;
    ///
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let limiter = RateLimiter::new(RateLimitConfig::per_second(5));
    /// let ip: IpAddr = FromStr::from_str("127.0.0.1").unwrap();
    ///
    /// let result = limiter.check_rate_limit(&ip).await;
    /// assert!(result.is_allowed());
    /// # });
    /// ```
    pub async fn check_rate_limit(&self, ip: &IpAddr) -> RateLimitResult {
        // If rate limiting is effectively disabled, always allow
        if self.config.requests_per_second >= 1_000_000.0 {
            return RateLimitResult::Allowed;
        }

        let now = Instant::now();

        // Periodic cleanup of stale entries
        {
            let mut last_cleanup = self.last_cleanup.write().await;
            if now.duration_since(*last_cleanup) >= self.config.cleanup_interval {
                self.cleanup_stale_entries(now).await;
                *last_cleanup = now;
            }
        }

        // Get or create token bucket for this IP
        let mut buckets = self.buckets.write().await;
        let bucket = buckets.entry(*ip).or_insert_with(|| {
            TokenBucket::new(self.config.burst_size)
        });

        // Try to consume a token
        if bucket.try_consume(&self.config, now) {
            RateLimitResult::Allowed
        } else {
            let retry_after = bucket.time_until_next_token(&self.config);
            RateLimitResult::RateLimited { retry_after }
        }
    }

    /// Cleans up stale entries that haven't been accessed recently.
    ///
    /// # Arguments
    ///
    /// * `now` - Current time
    async fn cleanup_stale_entries(&self, now: Instant) {
        let mut buckets = self.buckets.write().await;
        buckets.retain(|_, bucket| {
            now.duration_since(bucket.last_update) < self.config.entry_ttl
        });
    }

    /// Returns the number of IP addresses currently being tracked.
    ///
    /// This is primarily useful for testing and monitoring.
    ///
    /// # Returns
    ///
    /// The number of active IP entries
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
    /// use std::net::IpAddr;
    /// use std::str::FromStr;
    ///
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let limiter = RateLimiter::new(RateLimitConfig::per_second(10));
    /// assert_eq!(limiter.tracked_ip_count().await, 0);
    ///
    /// let ip1: IpAddr = FromStr::from_str("127.0.0.1").unwrap();
    /// limiter.check_rate_limit(&ip1).await;
    /// assert_eq!(limiter.tracked_ip_count().await, 1);
    /// # });
    /// ```
    pub async fn tracked_ip_count(&self) -> usize {
        self.buckets.read().await.len()
    }

    /// Returns whether rate limiting is effectively enabled.
    ///
    /// Rate limiting is considered disabled if the rate is set to a very high value
    /// (>= 1,000,000 requests per second), which means all requests will be allowed.
    ///
    /// # Returns
    ///
    /// `true` if rate limiting is enabled, `false` if disabled
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
    ///
    /// // Disabled rate limiter (default)
    /// let disabled = RateLimiter::disabled();
    /// assert!(!disabled.is_enabled());
    ///
    /// // Enabled rate limiter
    /// let enabled = RateLimiter::new(RateLimitConfig::per_second(10.0));
    /// assert!(enabled.is_enabled());
    /// ```
    pub fn is_enabled(&self) -> bool {
        self.config.requests_per_second < 1_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn create_test_ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    }

    #[test]
    fn test_rate_limit_config_new() {
        let config = RateLimitConfig::new(10.0, 20);
        assert_eq!(config.requests_per_second, 10.0);
        assert_eq!(config.burst_size, 20);
    }

    #[test]
    fn test_rate_limit_config_per_second() {
        let config = RateLimitConfig::per_second(10.0);
        assert_eq!(config.requests_per_second, 10.0);
        assert_eq!(config.burst_size, 20); // 2x rate
    }

    #[test]
    fn test_rate_limit_config_per_minute() {
        let config = RateLimitConfig::per_minute(600);
        assert_eq!(config.requests_per_second, 10.0); // 600 / 60
    }

    #[test]
    fn test_rate_limit_config_default() {
        let config = RateLimitConfig::default();
        assert_eq!(config.requests_per_second, f64::MAX);
        assert_eq!(config.burst_size, u32::MAX);
    }

    #[tokio::test]
    async fn test_rate_limiter_disabled() {
        let limiter = RateLimiter::disabled();
        let ip = create_test_ip();

        // Should always allow requests
        for _ in 0..1000 {
            assert!(limiter.check_rate_limit(&ip).await.is_allowed());
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let config = RateLimitConfig::new(10.0, 10); // 10 rps, burst 10
        let limiter = RateLimiter::new(config);
        let ip = create_test_ip();

        // First 10 requests should be allowed (burst)
        for _ in 0..10 {
            assert!(limiter.check_rate_limit(&ip).await.is_allowed());
        }

        // 11th request should be rate limited
        assert!(!limiter.check_rate_limit(&ip).await.is_allowed());
    }

    #[tokio::test]
    async fn test_rate_limiter_token_refill() {
        let config = RateLimitConfig::new(10.0, 10); // 10 rps, burst 10
        let limiter = RateLimiter::new(config);
        let ip = create_test_ip();

        // Exhaust burst
        for _ in 0..10 {
            assert!(limiter.check_rate_limit(&ip).await.is_allowed());
        }
        assert!(!limiter.check_rate_limit(&ip).await.is_allowed());

        // Wait for token refill (0.11 seconds = 1 token + buffer)
        tokio::time::sleep(Duration::from_millis(110)).await;

        // Should have 1 token available
        assert!(limiter.check_rate_limit(&ip).await.is_allowed());
    }

    #[tokio::test]
    async fn test_rate_limiter_multiple_ips() {
        let config = RateLimitConfig::new(5.0, 5);
        let limiter = RateLimiter::new(config);

        let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));

        // Each IP should have its own rate limit
        for _ in 0..5 {
            assert!(limiter.check_rate_limit(&ip1).await.is_allowed());
            assert!(limiter.check_rate_limit(&ip2).await.is_allowed());
        }

        // Both should be rate limited now
        assert!(!limiter.check_rate_limit(&ip1).await.is_allowed());
        assert!(!limiter.check_rate_limit(&ip2).await.is_allowed());
    }

    #[tokio::test]
    async fn test_rate_limiter_tracked_ip_count() {
        let limiter = RateLimiter::new(RateLimitConfig::per_second(10.0));

        assert_eq!(limiter.tracked_ip_count().await, 0);

        let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        limiter.check_rate_limit(&ip1).await;

        assert_eq!(limiter.tracked_ip_count().await, 1);

        let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));
        limiter.check_rate_limit(&ip2).await;

        assert_eq!(limiter.tracked_ip_count().await, 2);
    }

    #[tokio::test]
    async fn test_rate_limiter_retry_after() {
        let config = RateLimitConfig::new(10.0, 1); // Small burst for testing
        let limiter = RateLimiter::new(config);
        let ip = create_test_ip();

        // Consume the only token
        assert!(limiter.check_rate_limit(&ip).await.is_allowed());

        // Next request should be rate limited
        let result = limiter.check_rate_limit(&ip).await;
        assert!(!result.is_allowed());
        assert!(result.retry_after().is_some());

        // Retry after should be roughly 0.1 seconds (1 token / 10 rps)
        let retry_after = result.retry_after().unwrap();
        assert!(retry_after.as_millis() >= 90);
        assert!(retry_after.as_millis() <= 110);
    }

    #[tokio::test]
    async fn test_token_bucket_new() {
        let bucket = TokenBucket::new(10);
        assert_eq!(bucket.tokens, 10.0);
    }

    #[tokio::test]
    async fn test_token_bucket_try_consume() {
        let config = RateLimitConfig::new(10.0, 10);
        let mut bucket = TokenBucket::new(10);
        let now = Instant::now();

        // Consume all tokens
        for _ in 0..10 {
            assert!(bucket.try_consume(&config, now));
        }

        // No more tokens
        assert!(!bucket.try_consume(&config, now));
    }

    #[tokio::test]
    async fn test_token_bucket_refill() {
        let config = RateLimitConfig::new(10.0, 10);
        let mut bucket = TokenBucket::new(1); // Start with 1 token
        let now = Instant::now();

        // Consume the token
        assert!(bucket.try_consume(&config, now));
        assert!(bucket.tokens < 1e-6, "tokens should be near 0, got {}", bucket.tokens);

        // Wait 0.11 seconds for 1 token to refill
        let _later = now + Duration::from_millis(110);
        assert!(bucket.try_consume(&config, _later));
    }

    #[tokio::test]
    async fn test_token_bucket_time_until_next_token() {
        let config = RateLimitConfig::new(10.0, 10);
        let mut bucket = TokenBucket::new(0); // Start empty
        let now = Instant::now();

        // No tokens available
        let time_to_wait = bucket.time_until_next_token(&config);
        assert!(time_to_wait.as_secs_f64() > 0.0);
        assert!(time_to_wait.as_secs_f64() <= 0.2); // Should be ~0.1 seconds

        // After waiting, should have tokens
        let _later = now + time_to_wait;
        bucket.tokens = 5.0; // Simulate tokens added
        assert_eq!(bucket.time_until_next_token(&config), Duration::ZERO);
    }
}
