use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse};
use madrpc_common::protocol::error::{Result, MadrpcError};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use hyper::{Request as HttpRequest, Method};
use hyper::body::Bytes;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use http_body_util::{Full, BodyExt};
use std::sync::Arc;

/// Global counter for generating unique JSON-RPC request IDs
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// MaDRPC client for making RPC calls over HTTP.
///
/// The client provides a high-level interface for making JSON-RPC 2.0 calls to MaDRPC
/// orchestrators via HTTP. It handles automatic retries with exponential backoff and error
/// classification.
///
/// # Architecture
///
/// The client uses hyper's HTTP client with HTTP/1.1 keep-alive for efficient connection
/// reuse. When making a request:
///
/// 1. A JSON-RPC 2.0 request is created with the method and parameters
/// 2. An HTTP POST request is sent to the orchestrator
/// 3. The JSON-RPC response is received and parsed
/// 4. The result is returned or an error is raised
///
/// # Retries
///
/// Transient errors (network issues, timeouts) are automatically retried with exponential
/// backoff and jitter. Permanent errors (invalid requests, JavaScript execution errors) fail
/// immediately.
///
/// # Example
///
/// ```rust,no_run
/// use madrpc_client::MadrpcClient;
/// use serde_json::json;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let client = MadrpcClient::new("http://127.0.0.1:8080").await?;
/// let result = client.call("compute", json!({"n": 42})).await?;
/// # Ok(())
/// # }
/// ```
pub struct MadrpcClient {
    base_url: String,
    http_client: Arc<Client<HttpConnector, Full<Bytes>>>,
    retry_config: RetryConfig,
}

impl MadrpcClient {
    /// Creates a new client connected to an orchestrator.
    ///
    /// This uses default configuration for retry logic.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the orchestrator (e.g., "http://127.0.0.1:8080")
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the client or an error if HTTP client creation fails.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::MadrpcClient;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = MadrpcClient::new("http://127.0.0.1:8080").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(base_url: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into();
        let http_client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(90))
            .build_http();

        let retry_config = RetryConfig::default();

        Ok(Self {
            base_url,
            http_client: Arc::new(http_client),
            retry_config,
        })
    }

    /// Creates a new client with custom retry configuration.
    ///
    /// Use this when you need full control over retry behavior.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the orchestrator (e.g., "http://127.0.0.1:8080")
    /// * `retry_config` - The retry configuration
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::{MadrpcClient, RetryConfig};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let retry_config = RetryConfig::new(5, 200, 10000, 2.0);
    /// let client = MadrpcClient::with_retry_config(
    ///     "http://127.0.0.1:8080",
    ///     retry_config
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_retry_config(
        base_url: impl Into<String>,
        retry_config: RetryConfig,
    ) -> Result<Self> {
        let base_url = base_url.into();
        let http_client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(Duration::from_secs(90))
            .build_http();

        Ok(Self {
            base_url,
            http_client: Arc::new(http_client),
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
    /// let client = MadrpcClient::new("http://127.0.0.1:8080")
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
    /// This method sends a JSON-RPC 2.0 request via HTTP POST and implements automatic
    /// retry logic with exponential backoff for transient failures.
    ///
    /// # Retry Behavior
    ///
    /// - Transient errors (network issues, timeouts) are automatically retried up to
    ///   `max_attempts` times
    /// - Non-retryable errors (invalid requests, JSON-RPC errors) fail immediately
    /// - Retries use exponential backoff with jitter to avoid thundering herd problems
    ///
    /// # Arguments
    ///
    /// * `method` - The method name to call (must be registered on the compute nodes)
    /// * `params` - The method parameters as a JSON value
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the JSON response from the RPC call, or an error if:
    /// - All retry attempts are exhausted
    /// - A non-retryable error occurs
    /// - The HTTP request fails
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The method is not found on any compute node
    /// - JavaScript execution fails on the node
    /// - Network connectivity is lost after all retries
    /// - The request parameters are invalid
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use madrpc_client::MadrpcClient;
    /// use serde_json::json;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = MadrpcClient::new("http://127.0.0.1:8080").await?;
    ///
    /// // Call a method with parameters
    /// let result = client.call("compute", json!({"n": 42})).await?;
    /// println!("Result: {}", result);
    ///
    /// // Call a method with no parameters
    /// let status = client.call("status", json!(null)).await?;
    /// println!("Status: {}", status);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call(
        &self,
        method: impl Into<String>,
        params: Value,
    ) -> Result<Value> {
        let method = method.into();

        // Generate unique request ID
        let request_id = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

        // Create JSON-RPC 2.0 request
        let jsonrpc_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.clone(),
            params,
            id: Value::Number(request_id.into()),
        };

        let mut last_error = None;

        for attempt in 1..=self.retry_config.max_attempts {
            // Log retry attempt
            if attempt > 1 {
                tracing::info!(
                    attempt = attempt,
                    max_attempts = self.retry_config.max_attempts,
                    method = %method,
                    "Retrying RPC call"
                );
            }

            // Try to execute the request
            match self.try_call(&jsonrpc_req).await {
                Ok(result) => {
                    // Success - return the result
                    if attempt > 1 {
                        tracing::info!(
                            attempt = attempt,
                            method = %method,
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
                            method = %method,
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
    /// 1. Creates an HTTP POST request with the JSON-RPC payload
    /// 2. Sends the request via hyper
    /// 3. Receives and parses the JSON-RPC response
    /// 4. Returns the result or an appropriate error
    ///
    /// # Arguments
    ///
    /// * `request` - The JSON-RPC request to execute
    async fn try_call(&self, request: &JsonRpcRequest) -> Result<Value> {
        // Serialize the JSON-RPC request
        let body = serde_json::to_vec(request)
            .map_err(|e| MadrpcError::JsonSerialization(e))?;

        // Build HTTP request
        let http_req = HttpRequest::builder()
            .method(Method::POST)
            .uri(&self.base_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| MadrpcError::InvalidRequest(format!("Invalid HTTP request: {}", e)))?;

        // Send the HTTP request
        let http_response = self.http_client
            .request(http_req)
            .await
            .map_err(|e| {
                // Hyper errors are typically network-related and retryable
                MadrpcError::Transport(format!("HTTP request failed: {}", e))
            })?;

        // Check HTTP status code
        let status = http_response.status();
        if !status.is_success() {
            // HTTP errors are retryable for server errors (5xx)
            if status.is_server_error() {
                return Err(MadrpcError::Transport(format!("HTTP error: {}", status)));
            } else {
                return Err(MadrpcError::InvalidResponse(format!(
                    "HTTP error: {}",
                    status
                )));
            }
        }

        // Collect the response body
        let whole_body = http_response
            .into_body()
            .collect()
            .await
            .map_err(|e| {
                MadrpcError::Transport(format!("Failed to read response body: {}", e))
            })?
            .to_bytes();

        // Parse the JSON-RPC response
        let jsonrpc_res: JsonRpcResponse = serde_json::from_slice(&whole_body)
            .map_err(|e| {
                // JSON parsing errors indicate an invalid response
                MadrpcError::InvalidResponse(format!("Failed to parse JSON-RPC response: {}", e))
            })?;

        // Check if the response is an error
        if let Some(jsonrpc_error) = jsonrpc_res.error {
            // Convert JSON-RPC error to MadrpcError
            let err = match jsonrpc_error.code {
                code if code <= -32000 => {
                    // Server error - these are typically retryable if they're internal errors
                    if jsonrpc_error.message.contains("timeout") ||
                       jsonrpc_error.message.contains("unavailable") {
                        MadrpcError::NodeUnavailable(jsonrpc_error.message)
                    } else {
                        MadrpcError::JavaScriptExecution(jsonrpc_error.message)
                    }
                }
                -32601 => MadrpcError::InvalidRequest(format!("Method not found: {}", jsonrpc_error.message)),
                -32602 => MadrpcError::InvalidRequest(format!("Invalid params: {}", jsonrpc_error.message)),
                -32600 => MadrpcError::InvalidResponse(jsonrpc_error.message),
                -32700 => MadrpcError::InvalidResponse(jsonrpc_error.message),
                _ => MadrpcError::JavaScriptExecution(jsonrpc_error.message),
            };
            return Err(err);
        }

        // Return the result (null results are valid in JSON-RPC)
        Ok(jsonrpc_res.result.unwrap_or(Value::Null))
    }
}

/// Clone implementation for `MadrpcClient`.
///
/// Cloning a client creates a new handle that shares the same underlying HTTP client
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
/// let client = MadrpcClient::new("http://127.0.0.1:8080").await?;
/// let client2 = client.clone(); // Shares the same HTTP client
///
/// // Both clients can make concurrent requests
/// let task1 = client.call("method1", serde_json::json!(null));
/// let task2 = client2.call("method2", serde_json::json!(null));
///
/// let (result1, result2) = tokio::join!(task1, task2);
/// # Ok(())
/// # }
/// ```
impl Clone for MadrpcClient {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            http_client: Arc::clone(&self.http_client),
            retry_config: self.retry_config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_creation() {
        let client = MadrpcClient::new("http://localhost:8080").await;
        // Will create successfully even if server doesn't exist
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_client_is_clonable() {
        let client = MadrpcClient::new("http://localhost:8080").await.unwrap();
        let client2 = client.clone();
        assert_eq!(client.base_url, client2.base_url);
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

        // We can't test the full client without a server, but we can test the builder
        let base_url = "http://localhost:8080";
        let http_client = Client::builder(TokioExecutor::new()).build_http();
        let _ = MadrpcClient {
            base_url: base_url.to_string(),
            http_client: Arc::new(http_client),
            retry_config,
        };
    }
}
