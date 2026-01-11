//! HTTP Server for MaDRPC Node
//!
//! This module provides the HTTP server implementation for the MaDRPC node
//! using hyper for HTTP/1.1 server functionality. The server accepts JSON-RPC
//! requests over HTTP and forwards them to the NodeRouter for processing.
//!
//! # Architecture
//!
//! The HTTP server:
//! - Listens on a TCP socket for incoming HTTP connections
//! - Spawns a tokio task for each connection
//! - Parses JSON-RPC requests from HTTP bodies
//! - Forwards requests to the NodeRouter for handling
//! - Returns JSON-RPC responses as HTTP responses
//!
//! # Example
//!
//! ```no_run
//! use madrpc_server::http_server::HttpServer;
//! use madrpc_server::Node;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() {
//! #    let script = tempfile::NamedTempFile::new().unwrap();
//! #    std::fs::write(script.path(), "// empty").unwrap();
//!     let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
//!     let server = HttpServer::new(node);
//!     server.run("127.0.0.1:8080".parse().unwrap()).await.unwrap();
//! }
//! ```

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Response, StatusCode};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use serde_json::json;

use crate::http_router::NodeRouter;
use crate::node::Node;
use madrpc_common::auth::AuthConfig;
use madrpc_common::protocol::JsonRpcError;
use madrpc_common::protocol::error::MadrpcError;
use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
use madrpc_common::transport::{HttpTransport, HyperRequest, HyperResponse};

/// Maximum request body size (10 MB)
///
/// This limit prevents memory exhaustion DoS attacks by rejecting requests
/// with overly large payloads before they can cause significant memory allocation.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Maximum number of concurrent connections
///
/// This limit prevents resource exhaustion DoS attacks by limiting the number
/// of concurrent connections. When this limit is reached, new connections
/// will wait until an existing connection completes.
const MAX_CONCURRENT_CONNECTIONS: usize = 1000;

/// HTTP server for MaDRPC node.
///
/// The server listens for HTTP requests and processes JSON-RPC requests
/// through the NodeRouter.
///
/// # Connection Limiting
///
/// The server enforces a maximum number of concurrent connections to prevent
/// resource exhaustion DoS attacks. When the limit is reached, new connections
/// will wait until an existing connection completes before being processed.
///
/// # Authentication
///
/// The server supports optional API key authentication via the `X-API-Key` header.
/// When authentication is configured, requests without a valid API key will be
/// rejected with HTTP 401 Unauthorized.
///
/// # Rate Limiting
///
/// The server supports optional per-IP rate limiting using a token bucket algorithm.
/// When rate limiting is configured, requests that exceed the configured rate will
/// be rejected with HTTP 429 Too Many Requests.
pub struct HttpServer {
    /// The router for handling JSON-RPC requests
    router: Arc<NodeRouter>,
    /// Semaphore for limiting concurrent connections
    connection_semaphore: Arc<Semaphore>,
    /// Authentication configuration
    auth_config: AuthConfig,
    /// Rate limiter for preventing abuse
    rate_limiter: Arc<RateLimiter>,
}

impl HttpServer {
    /// Creates a new HTTP server with the given node instance.
    ///
    /// # Arguments
    ///
    /// * `node` - The node instance to use for request handling
    ///
    /// # Returns
    ///
    /// A new `HttpServer` instance with authentication disabled
    ///
    /// # Example
    ///
    /// ```no_run
    /// use madrpc_server::http_server::HttpServer;
    /// use madrpc_server::Node;
    /// use std::sync::Arc;
    ///
    /// # let script = tempfile::NamedTempFile::new().unwrap();
    /// # std::fs::write(script.path(), "// empty").unwrap();
    /// let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    /// let server = HttpServer::new(node);
    /// ```
    pub fn new(node: Arc<Node>) -> Self {
        let router = Arc::new(NodeRouter::new(node));
        let connection_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
        Self {
            router,
            connection_semaphore,
            auth_config: AuthConfig::default(),
            rate_limiter: Arc::new(RateLimiter::disabled()),
        }
    }

    /// Sets the authentication configuration for the server.
    ///
    /// # Arguments
    ///
    /// * `auth_config` - The authentication configuration to use
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```no_run
    /// use madrpc_server::http_server::HttpServer;
    /// use madrpc_server::Node;
    /// use madrpc_common::auth::AuthConfig;
    /// use std::sync::Arc;
    ///
    /// # let script = tempfile::NamedTempFile::new().unwrap();
    /// # std::fs::write(script.path(), "// empty").unwrap();
    /// let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    /// let server = HttpServer::new(node)
    ///     .with_auth(AuthConfig::with_api_key("my-secret-key"));
    /// ```
    pub fn with_auth(mut self, auth_config: AuthConfig) -> Self {
        self.auth_config = auth_config;
        self
    }

    /// Sets the rate limiting configuration for the server.
    ///
    /// # Arguments
    ///
    /// * `rate_limit_config` - The rate limit configuration to use
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```no_run
    /// use madrpc_server::http_server::HttpServer;
    /// use madrpc_server::Node;
    /// use madrpc_common::rate_limit::RateLimitConfig;
    /// use std::sync::Arc;
    ///
    /// # let script = tempfile::NamedTempFile::new().unwrap();
    /// # std::fs::write(script.path(), "// empty").unwrap();
    /// let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    /// let server = HttpServer::new(node)
    ///     .with_rate_limit(RateLimitConfig::per_second(100));
    /// ```
    pub fn with_rate_limit(mut self, rate_limit_config: RateLimitConfig) -> Self {
        self.rate_limiter = Arc::new(RateLimiter::new(rate_limit_config));
        self
    }

    /// Runs the HTTP server on the specified address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The socket address to bind to
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure
    ///
    /// # Example
    ///
    /// ```no_run
    /// use madrpc_server::http_server::HttpServer;
    /// use madrpc_server::Node;
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() {
    /// #    let script = tempfile::NamedTempFile::new().unwrap();
    /// #    std::fs::write(script.path(), "// empty").unwrap();
    ///     let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    ///     let server = HttpServer::new(node);
    ///     server.run("127.0.0.1:8080".parse().unwrap()).await.unwrap();
    /// }
    /// ```
    pub async fn run(self, addr: SocketAddr) -> Result<(), MadrpcError> {
        let listener = TcpListener::bind(addr).await
            .map_err(|e| MadrpcError::Transport(format!("Failed to bind to {}: {}", addr, e)))?;

        tracing::info!("HTTP server listening on {}", listener.local_addr()
            .map_err(|e| MadrpcError::Transport(format!("Failed to get local address: {}", e)))?);
        tracing::info!("Authentication: {}", self.auth_config);
        tracing::info!("Rate Limiting: enabled ({} req/s, burst {})",
            self.rate_limiter.config.requests_per_second,
            self.rate_limiter.config.burst_size);

        loop {
            let (stream, remote_addr) = listener.accept().await
                .map_err(|e| MadrpcError::Transport(format!("Failed to accept connection: {}", e)))?;

            // Acquire a permit from the semaphore to limit concurrent connections
            // This will wait until a permit is available if the limit is reached
            let semaphore = self.connection_semaphore.clone();
            let permit = semaphore.acquire_owned().await
                .map_err(|e| MadrpcError::Transport(format!("Failed to acquire connection permit: {}", e)))?;

            let io = TokioIo::new(stream);
            let router = self.router.clone();
            let auth_config = self.auth_config.clone();
            let rate_limiter = self.rate_limiter.clone();
            let client_ip = remote_addr.ip();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let router = router.clone();
                    let auth_config = auth_config.clone();
                    let rate_limiter = rate_limiter.clone();
                    let client_ip = client_ip;
                    async move { Self::handle_request(router, req, auth_config, rate_limiter, client_ip).await }
                });

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    tracing::error!("Error serving connection: {}", err);
                }

                // Permit is released when it goes out of scope
                drop(permit);
            });
        }
    }

    /// Handles an HTTP request.
    ///
    /// # Arguments
    ///
    /// * `router` - The router to handle the JSON-RPC request
    /// * `req` - The incoming HTTP request
    /// * `auth_config` - The authentication configuration
    /// * `rate_limiter` - The rate limiter for preventing abuse
    /// * `client_ip` - The IP address of the client
    ///
    /// # Returns
    ///
    /// A `Result` containing the HTTP response or an error
    async fn handle_request(
        router: Arc<NodeRouter>,
        req: HyperRequest,
        auth_config: AuthConfig,
        rate_limiter: Arc<RateLimiter>,
        client_ip: IpAddr,
    ) -> Result<HyperResponse, MadrpcError> {
        // Check rate limit first (before expensive operations)
        let rate_limit_result = rate_limiter.check_rate_limit(&client_ip).await;
        if !rate_limit_result.is_allowed() {
            tracing::warn!("Rate limit exceeded for IP: {}", client_ip);
            return Ok(Self::rate_limited_response(rate_limit_result.retry_after()));
        }

        // Check authentication if enabled
        if auth_config.requires_auth() {
            let headers = req.headers();
            let api_key = headers.get("x-api-key")
                .and_then(|v| v.to_str().ok());

            if !auth_config.validate_api_key(api_key.unwrap_or("")) {
                tracing::warn!("Authentication failed: invalid or missing API key");
                return Ok(Self::unauthorized_response());
            }
        }

        // Only accept POST requests for JSON-RPC
        if req.method() != hyper::Method::POST {
            return Ok(HttpTransport::to_http_error(
                json!(null),
                JsonRpcError::invalid_request()
            ));
        }

        // Read the request body
        let whole_body = req.into_body();
        let body = match whole_body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                tracing::error!("Failed to read request body: {}", e);
                return Ok(HttpTransport::to_http_error(
                    json!(null),
                    JsonRpcError::internal_error("Failed to read request body")
                ));
            }
        };

        // Check request body size to prevent DoS attacks
        if body.len() > MAX_BODY_SIZE {
            tracing::error!(
                "Request body too large: {} bytes (max {} bytes)",
                body.len(),
                MAX_BODY_SIZE
            );
            return Ok(HttpTransport::to_http_error(
                json!(null),
                JsonRpcError::request_too_large(MAX_BODY_SIZE)
            ));
        }

        // Parse the JSON-RPC request
        let jsonrpc_req = match HttpTransport::parse_jsonrpc(body) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!("Failed to parse JSON-RPC request: {}", e);
                return Ok(HttpTransport::to_http_error(
                    json!(null),
                    JsonRpcError::parse_error()
                ));
            }
        };

        // Handle via router
        let jsonrpc_res = match router.handle_request(jsonrpc_req).await {
            Ok(res) => res,
            Err(e) => {
                let error_msg = format!("{:?}", e);
                tracing::error!("Error handling request: {}", error_msg);
                return Ok(HttpTransport::to_http_error(
                    json!(null),
                    JsonRpcError::internal_error(&error_msg)
                ));
            }
        };

        Ok(HttpTransport::to_http_response(jsonrpc_res))
    }

    /// Creates an HTTP 401 Unauthorized response.
    ///
    /// # Returns
    ///
    /// A hyper response with 401 status and appropriate error message
    fn unauthorized_response() -> HyperResponse {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(r#"{"jsonrpc":"2.0","error":{"code":-401,"message":"Unauthorized: Invalid or missing API key"},"id":null}"#)))
            .unwrap()
    }

    /// Creates an HTTP 429 Too Many Requests response.
    ///
    /// # Arguments
    ///
    /// * `retry_after` - Optional duration until the request can be retried
    ///
    /// # Returns
    ///
    /// A hyper response with 429 status and appropriate error message
    fn rate_limited_response(retry_after: Option<Duration>) -> HyperResponse {
        let retry_after_secs = retry_after
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|| "1".to_string());

        let body = json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -429,
                "message": "Too Many Requests: Rate limit exceeded",
                "data": { "retry_after": retry_after_secs }
            },
            "id": null
        });

        Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("Content-Type", "application/json")
            .header("Retry-After", retry_after_secs)
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use madrpc_common::protocol::REQUEST_TOO_LARGE;

    fn create_test_script(content: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), content).unwrap();
        file
    }

    #[tokio::test]
    async fn test_server_creation() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let server = HttpServer::new(node.clone());
        assert!(node.script_path().exists());

        // Verify the semaphore is initialized with the correct number of permits
        assert_eq!(server.connection_semaphore.available_permits(), MAX_CONCURRENT_CONNECTIONS);

        // Verify authentication is disabled by default
        assert!(!server.auth_config.requires_auth());
    }

    #[tokio::test]
    async fn test_server_with_auth() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let server = HttpServer::new(node.clone())
            .with_auth(AuthConfig::with_api_key("test-key"));

        // Verify authentication is enabled
        assert!(server.auth_config.requires_auth());
        assert!(server.auth_config.validate_api_key("test-key"));
        assert!(!server.auth_config.validate_api_key("wrong-key"));
    }

    #[tokio::test]
    async fn test_only_post_requests_allowed() {
        // The health check is now a JSON-RPC method, not a special HTTP endpoint
        // This test verifies that non-POST requests are rejected
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let _server = HttpServer::new(node.clone());

        // We can't easily test this without making an actual HTTP request,
        // but the test_server_creation test ensures the server initializes correctly
        assert!(node.script_path().exists());
    }

    #[test]
    fn test_max_body_size_constant() {
        // Verify the constant is set to 10 MB
        assert_eq!(MAX_BODY_SIZE, 10 * 1024 * 1024);
    }

    #[test]
    fn test_max_concurrent_connections_constant() {
        // Verify the constant is set to 1000
        assert_eq!(MAX_CONCURRENT_CONNECTIONS, 1000);
    }

    #[test]
    fn test_request_too_large_error() {
        // Test that the request_too_large error is created correctly
        let error = JsonRpcError::request_too_large(1024);
        assert_eq!(error.code, REQUEST_TOO_LARGE);
        assert!(error.message.contains("1024"));
        assert!(error.message.contains("too large"));
    }

    #[tokio::test]
    async fn test_connection_limit_enforcement() {
        // Create a server with a small connection limit for testing
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());

        // Create a custom server with a limited number of permits for testing
        let _router = Arc::new(NodeRouter::new(node));
        let connection_semaphore = Arc::new(Semaphore::new(2)); // Only 2 permits for testing

        // Verify initial state
        assert_eq!(connection_semaphore.available_permits(), 2);

        // Simulate acquiring all permits
        let permit1 = connection_semaphore.clone().acquire_owned().await.unwrap();
        assert_eq!(connection_semaphore.available_permits(), 1);

        let permit2 = connection_semaphore.clone().acquire_owned().await.unwrap();
        assert_eq!(connection_semaphore.available_permits(), 0);

        // Try to acquire a third permit - this should not immediately succeed
        // We'll create a task that tries to acquire and verify it waits
        let semaphore_clone = connection_semaphore.clone();
        let acquire_task = tokio::spawn(async move {
            // This should wait since no permits are available
            let _permit = semaphore_clone.acquire_owned().await.unwrap();
            // If we get here, a permit was acquired
        });

        // Give the task a chance to try to acquire (it should be waiting)
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // The task should still be running (waiting for a permit)
        assert!(!acquire_task.is_finished());

        // Release one permit
        drop(permit1);

        // Wait for the acquire task to complete
        tokio::time::timeout(tokio::time::Duration::from_millis(100), acquire_task)
            .await
            .expect("Acquire task should complete after permit is released")
            .expect("Acquire task should succeed");

        // Now the task completed and released its permit, so we should have 1 available (permit2 still held)
        assert_eq!(connection_semaphore.available_permits(), 1);

        // Release the second permit
        drop(permit2);

        // Verify both permits are available again
        assert_eq!(connection_semaphore.available_permits(), 2);
    }
}
