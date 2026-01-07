use madrpc_common::protocol::Request;
use madrpc_common::protocol::error::{Result, MadrpcError};
use madrpc_common::transport::TcpTransportAsync;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use crate::pool::{ConnectionPool, PoolConfig};

/// Retry configuration for RPC calls.
///
/// This struct controls the retry behavior for transient failures. Retries use
/// exponential backoff with jitter to avoid thundering herd problems when multiple
/// clients experience similar failures.
///
/// # Fields
///
/// - `max_attempts`: Maximum number of retry attempts (including the initial attempt)
/// - `base_delay_ms`: Base delay in milliseconds for exponential backoff
/// - `max_delay_ms`: Maximum delay cap in milliseconds
/// - `backoff_multiplier`: Exponential backoff multiplier
///
/// # Default Configuration
///
/// The default configuration is:
/// - `max_attempts`: 3
/// - `base_delay_ms`: 100ms
/// - `max_delay_ms`: 5000ms
/// - `backoff_multiplier`: 2.0
///
/// # Example
///
/// ```rust
/// use madrpc_client::RetryConfig;
///
/// // Custom retry configuration: up to 5 attempts with longer delays
/// let config = RetryConfig::new(5, 200, 10000, 2.0);
/// ```
#[derive(Clone, Debug)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (including the initial attempt)
    pub max_attempts: u32,
    /// Base delay in milliseconds for exponential backoff
    pub base_delay_ms: u64,
    /// Maximum delay cap in milliseconds
    pub max_delay_ms: u64,
    /// Exponential backoff multiplier
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 100,
            max_delay_ms: 5000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Creates a new retry config with custom settings.
    ///
    /// # Arguments
    ///
    /// * `max_attempts` - Maximum number of retry attempts (including initial attempt)
    /// * `base_delay_ms` - Base delay in milliseconds for exponential backoff
    /// * `max_delay_ms` - Maximum delay cap in milliseconds
    /// * `backoff_multiplier` - Exponential backoff multiplier (e.g., 2.0 doubles each time)
    ///
    /// # Example
    ///
    /// ```rust
    /// use madrpc_client::RetryConfig;
    ///
    /// let config = RetryConfig::new(5, 200, 10000, 2.0);
    /// ```
    pub fn new(max_attempts: u32, base_delay_ms: u64, max_delay_ms: u64, backoff_multiplier: f64) -> Self {
        Self {
            max_attempts,
            base_delay_ms,
            max_delay_ms,
            backoff_multiplier,
        }
    }

    /// Calculate delay for a given attempt using exponential backoff with jitter.
    ///
    /// The delay formula is: `min(base_delay_ms * multiplier^(attempt-1), max_delay_ms) + jitter`
    ///
    /// Jitter is added as +/- 10% of the calculated delay to prevent thundering herd
    /// problems when multiple clients retry simultaneously.
    ///
    /// # Arguments
    ///
    /// * `attempt` - The attempt number (1-indexed, starting at 1 for first retry)
    fn calculate_delay(&self, attempt: u32) -> Duration {
        let delay_ms = (self.base_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32 - 1))
            .min(self.max_delay_ms as f64) as u64;

        // Add small random jitter to avoid thundering herd
        let jitter = (delay_ms as f64 * 0.1) as u64;
        let jitter_amount = if jitter > 0 && rand::random::<bool>() {
            rand::random::<u64>() % jitter
        } else {
            0
        };

        Duration::from_millis(delay_ms + jitter_amount)
    }
}

/// MaDRPC client for making RPC calls.
///
/// The client provides a high-level interface for making RPC calls to MaDRPC orchestrators.
/// It handles connection pooling, automatic retries with exponential backoff, and error
/// classification.
///
/// # Architecture
///
/// The client maintains a connection pool to each orchestrator address, allowing multiple
/// concurrent requests to reuse existing TCP connections. When making a request:
///
/// 1. A connection is acquired from the pool (or created if needed)
/// 2. The request is sent to the orchestrator
/// 3. The response is received and decoded
/// 4. The connection is returned to the pool
///
/// # Retries
///
/// Transient errors (network issues, timeouts, node unavailability) are automatically
/// retried with exponential backoff and jitter. Permanent errors (invalid requests,
/// JavaScript execution errors) fail immediately.
///
/// # Example
///
/// ```rust,no_run
/// use madrpc_client::MadrpcClient;
/// use serde_json::json;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let client = MadrpcClient::new("127.0.0.1:8080").await?;
/// let result = client.call("compute", json!({"n": 42})).await?;
/// # Ok(())
/// # }
/// ```
pub struct MadrpcClient {
    orchestrator_addr: String,
    pool: Arc<ConnectionPool>,
    retry_config: RetryConfig,
}

impl MadrpcClient {
    /// Creates a new client connected to an orchestrator.
    ///
    /// This uses default configuration for both the connection pool and retry logic.
    ///
    /// # Arguments
    ///
    /// * `orchestrator_addr` - The orchestrator address (e.g., "127.0.0.1:8080")
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the client or an error if pool initialization fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::MadrpcClient;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = MadrpcClient::new("127.0.0.1:8080").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(orchestrator_addr: impl Into<String>) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let pool = Arc::new(ConnectionPool::new(PoolConfig::default())?);
        let retry_config = RetryConfig::default();

        Ok(Self {
            orchestrator_addr,
            pool,
            retry_config,
        })
    }

    /// Creates a new client with custom pool configuration.
    ///
    /// Use this when you need to control connection pool parameters such as maximum
    /// connections per address or acquisition timeout.
    ///
    /// # Arguments
    ///
    /// * `orchestrator_addr` - The orchestrator address (e.g., "127.0.0.1:8080")
    /// * `config` - The pool configuration
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::{MadrpcClient, PoolConfig};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = PoolConfig {
    ///     max_connections: 20,
    ///     acquire_timeout_ms: 60000,
    /// };
    /// let client = MadrpcClient::with_config("127.0.0.1:8080", config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_config(orchestrator_addr: impl Into<String>, config: PoolConfig) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let pool = Arc::new(ConnectionPool::new(config)?);
        let retry_config = RetryConfig::default();

        Ok(Self {
            orchestrator_addr,
            pool,
            retry_config,
        })
    }

    /// Creates a new client with custom pool and retry configuration.
    ///
    /// Use this when you need full control over both connection pool and retry behavior.
    ///
    /// # Arguments
    ///
    /// * `orchestrator_addr` - The orchestrator address (e.g., "127.0.0.1:8080")
    /// * `pool_config` - The pool configuration
    /// * `retry_config` - The retry configuration
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::{MadrpcClient, PoolConfig, RetryConfig};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let pool_config = PoolConfig {
    ///     max_connections: 20,
    ///     acquire_timeout_ms: 60000,
    /// };
    /// let retry_config = RetryConfig::new(5, 200, 10000, 2.0);
    /// let client = MadrpcClient::with_retry_config(
    ///     "127.0.0.1:8080",
    ///     pool_config,
    ///     retry_config
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_retry_config(
        orchestrator_addr: impl Into<String>,
        pool_config: PoolConfig,
        retry_config: RetryConfig,
    ) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let pool = Arc::new(ConnectionPool::new(pool_config)?);

        Ok(Self {
            orchestrator_addr,
            pool,
            retry_config,
        })
    }

    /// Sets retry configuration for this client.
    ///
    /// This is a builder-style method that consumes the client and returns a new one
    /// with the specified retry configuration.
    ///
    /// # Arguments
    ///
    /// * `retry_config` - The retry configuration
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::{MadrpcClient, RetryConfig};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = MadrpcClient::new("127.0.0.1:8080")
    ///     .await?
    ///     .with_retry(RetryConfig::new(5, 200, 10000, 2.0));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_retry(mut self, retry_config: RetryConfig) -> Self {
        self.retry_config = retry_config;
        self
    }

    /// Calls an RPC method.
    ///
    /// This method acquires a connection from the pool, sends the request, and returns
    /// the connection to the pool. It implements automatic retry logic with exponential
    /// backoff for transient failures.
    ///
    /// # Retry Behavior
    ///
    /// - Transient errors (network issues, timeouts, node unavailability) are automatically
    ///   retried up to `max_attempts` times
    /// - Non-retryable errors (invalid requests, JavaScript execution errors) fail immediately
    /// - Retries use exponential backoff with jitter to avoid thundering herd problems
    ///
    /// # Arguments
    ///
    /// * `method` - The method name to call (must be registered on the compute nodes)
    /// * `args` - The method arguments as a JSON value
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the JSON response from the RPC call, or an error if:
    /// - All retry attempts are exhausted
    /// - A non-retryable error occurs
    /// - The connection pool times out
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The method is not found on any compute node
    /// - JavaScript execution fails on the node
    /// - Network connectivity is lost after all retries
    /// - The connection pool times out acquiring a connection
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::MadrpcClient;
    /// use serde_json::json;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = MadrpcClient::new("127.0.0.1:8080").await?;
    ///
    /// // Call a method with arguments
    /// let result = client.call("compute", json!({"n": 42})).await?;
    /// println!("Result: {}", result);
    ///
    /// // Call a method with no arguments
    /// let status = client.call("status", json!({})).await?;
    /// println!("Status: {}", status);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call(
        &self,
        method: impl Into<String>,
        args: Value,
    ) -> Result<Value> {
        let request = Request::new(method, args);

        let mut last_error = None;

        for attempt in 1..=self.retry_config.max_attempts {
            // Log retry attempt
            if attempt > 1 {
                tracing::info!(
                    attempt = attempt,
                    max_attempts = self.retry_config.max_attempts,
                    "Retrying RPC call"
                );
            }

            // Try to execute the request
            match self.try_call(&request).await {
                Ok(result) => {
                    // Success - return the result
                    if attempt > 1 {
                        tracing::info!(
                            attempt = attempt,
                            "RPC call succeeded after retry"
                        );
                    }
                    return Ok(result);
                }
                Err(err) => {
                    // Check if error is retryable
                    if !err.is_retryable() {
                        // Non-retryable error - fail immediately
                        return Err(err);
                    }

                    // Store error for potential retry
                    last_error = Some(err);

                    // If this isn't the last attempt, wait before retrying
                    if attempt < self.retry_config.max_attempts {
                        let delay = self.retry_config.calculate_delay(attempt);
                        tracing::debug!(
                            attempt = attempt,
                            delay_ms = delay.as_millis(),
                            "Waiting before retry"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        // All attempts exhausted
        Err(last_error.unwrap_or_else(|| {
            MadrpcError::InvalidResponse("All retry attempts exhausted".to_string())
        }))
    }

    /// Internal method to execute a single RPC call attempt.
    ///
    /// This method:
    /// 1. Acquires a connection from the pool
    /// 2. Locks the stream for exclusive access
    /// 3. Sends the request via TCP transport
    /// 4. Receives and decodes the response
    /// 5. Returns the connection to the pool
    ///
    /// # Arguments
    ///
    /// * `request` - The RPC request to execute
    async fn try_call(&self, request: &Request) -> Result<Value> {
        // Acquire connection from pool
        let conn = self.pool.acquire(&self.orchestrator_addr).await?;

        // Lock the stream for this request
        let mut stream = conn.stream.lock().await;

        // Create transport for sending request
        let transport = TcpTransportAsync::new()?;

        // Send request and get response
        let response = transport.send_request(&mut stream, request).await?;

        // Release the stream lock
        drop(stream);

        // Return connection to pool
        self.pool.release(conn).await;

        // Handle response
        if response.success {
            response.result.ok_or_else(|| {
                MadrpcError::InvalidResponse("Missing result in success response".to_string())
            })
        } else {
            Err(MadrpcError::JavaScriptExecution(
                response.error.unwrap_or_else(|| "Unknown error".to_string())
            ))
        }
    }
}

/// Clone implementation for `MadrpcClient`.
///
/// Cloning a client creates a new handle that shares the same underlying connection pool
/// and retry configuration. This is useful for making concurrent requests from multiple
/// tasks or threads.
///
/// # Example
///
/// ```rust,no_run
/// use madrpc_client::MadrpcClient;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let client = MadrpcClient::new("127.0.0.1:8080").await?;
/// let client2 = client.clone(); // Shares the same connection pool
///
/// // Both clients can make concurrent requests
/// let task1 = client.call("method1", serde_json::json!({}));
/// let task2 = client2.call("method2", serde_json::json!({}));
///
/// let (result1, result2) = tokio::join!(task1, task2);
/// # Ok(())
/// # }
/// ```
impl Clone for MadrpcClient {
    fn clone(&self) -> Self {
        Self {
            orchestrator_addr: self.orchestrator_addr.clone(),
            pool: Arc::clone(&self.pool),
            retry_config: self.retry_config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use madrpc_common::protocol::error::MadrpcError;

    // Note: Tests require a running server
    // These are basic unit tests

    #[tokio::test]
    async fn test_client_creation() {
        let client = MadrpcClient::new("localhost:8080").await;
        // Will create successfully even if server doesn't exist
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_client_is_clonable() {
        let client = MadrpcClient::new("localhost:8080").await.unwrap();
        let client2 = client.clone();
        assert_eq!(client.orchestrator_addr, client2.orchestrator_addr);
    }

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.base_delay_ms, 100);
        assert_eq!(config.max_delay_ms, 5000);
        assert_eq!(config.backoff_multiplier, 2.0);
    }

    #[test]
    fn test_retry_config_custom() {
        let config = RetryConfig::new(5, 200, 10000, 3.0);
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.base_delay_ms, 200);
        assert_eq!(config.max_delay_ms, 10000);
        assert_eq!(config.backoff_multiplier, 3.0);
    }

    #[test]
    fn test_retry_delay_calculation() {
        let config = RetryConfig::new(3, 100, 5000, 2.0);

        // Attempt 1: 100ms * 2^0 = 100ms (plus jitter)
        let delay1 = config.calculate_delay(1);
        assert!(delay1.as_millis() >= 100);
        assert!(delay1.as_millis() < 115); // 100 + 10% jitter

        // Attempt 2: 100ms * 2^1 = 200ms (plus jitter)
        let delay2 = config.calculate_delay(2);
        assert!(delay2.as_millis() >= 200);
        assert!(delay2.as_millis() < 230); // 200 + 10% jitter

        // Attempt 3: 100ms * 2^2 = 400ms (plus jitter)
        let delay3 = config.calculate_delay(3);
        assert!(delay3.as_millis() >= 400);
        assert!(delay3.as_millis() < 460); // 400 + 10% jitter
    }

    #[test]
    fn test_retry_delay_max_cap() {
        let config = RetryConfig::new(10, 100, 200, 2.0);

        // Even with high attempt count, delay should be capped
        let delay = config.calculate_delay(10);
        assert!(delay.as_millis() <= 220); // 200 + 10% jitter
    }

    #[test]
    fn test_madrpc_error_is_retryable() {
        // Retryable errors
        assert!(MadrpcError::Transport("test".to_string()).is_retryable());
        assert!(MadrpcError::Timeout(1000).is_retryable());
        assert!(MadrpcError::NodeUnavailable("test".to_string()).is_retryable());
        assert!(MadrpcError::Connection("test".to_string()).is_retryable());
        assert!(MadrpcError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "test"
        )).is_retryable());

        // Non-retryable errors
        assert!(!MadrpcError::JavaScriptExecution("test".to_string()).is_retryable());
        assert!(!MadrpcError::InvalidResponse("test".to_string()).is_retryable());
        assert!(!MadrpcError::InvalidRequest("test".to_string()).is_retryable());
        assert!(!MadrpcError::AllNodesFailed.is_retryable());
    }

    #[test]
    fn test_client_with_retry() {
        let retry_config = RetryConfig::new(5, 50, 1000, 1.5);
        let config = PoolConfig::default();

        // We can't test the full client without a server, but we can test the builder
        let _ = MadrpcClient::with_retry_config("localhost:8080", config, retry_config);
    }
}
