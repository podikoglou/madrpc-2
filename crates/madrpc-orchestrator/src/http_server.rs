//! HTTP Server for Orchestrator
//!
//! This module provides the HTTP server implementation using axum.
//! It handles JSON-RPC requests and forwards them to the orchestrator router.

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use hyper::body::Bytes;
use serde_json::json;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::http_router::OrchestratorRouter;
use crate::orchestrator::Orchestrator;
use madrpc_common::auth::AuthConfig;
use madrpc_common::protocol::error::MadrpcError;
use madrpc_common::rate_limit::{RateLimiter, RateLimitConfig};
use madrpc_common::transport::HttpTransport;

/// HTTP server for the orchestrator.
///
/// This server:
/// - Handles JSON-RPC POST requests at `/`
/// - Provides a health check endpoint at `/__health`
/// - Uses the orchestrator router for request routing
/// - Supports optional API key authentication
/// - Supports optional per-IP rate limiting
pub struct HttpServer {
    /// Orchestrator router for handling requests
    router: Arc<OrchestratorRouter>,
    /// Authentication configuration
    auth_config: AuthConfig,
    /// Rate limiter for preventing abuse
    rate_limiter: Arc<RateLimiter>,
}

impl HttpServer {
    /// Creates a new HTTP server.
    ///
    /// # Arguments
    /// * `orchestrator` - Arc-wrapped orchestrator instance
    ///
    /// # Returns
    /// A new HTTP server instance with authentication and rate limiting disabled
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        let router = Arc::new(OrchestratorRouter::new(orchestrator));
        Self {
            router,
            auth_config: AuthConfig::default(),
            rate_limiter: Arc::new(RateLimiter::disabled()),
        }
    }

    /// Sets the authentication configuration for the server.
    ///
    /// # Arguments
    /// * `auth_config` - The authentication configuration to use
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_auth(mut self, auth_config: AuthConfig) -> Self {
        self.auth_config = auth_config;
        self
    }

    /// Sets the rate limiting configuration for the server.
    ///
    /// # Arguments
    /// * `rate_limit_config` - The rate limit configuration to use
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_rate_limit(mut self, rate_limit_config: RateLimitConfig) -> Self {
        self.rate_limiter = Arc::new(RateLimiter::new(rate_limit_config));
        self
    }

    /// Runs the HTTP server.
    ///
    /// # Arguments
    /// * `addr` - Socket address to bind to (e.g., "0.0.0.0:8080")
    ///
    /// # Returns
    /// - `Ok(())` - Server ran successfully
    /// - `Err(MadrpcError)` - Server failed to start or run
    ///
    /// # Behavior
    /// - Binds to the specified address
    /// - Logs the listening address
    /// - Runs indefinitely until shutdown
    pub async fn run(self, addr: SocketAddr) -> Result<(), MadrpcError> {
        // Log authentication configuration
        info!("Authentication: {}", self.auth_config);
        info!("Rate Limiting: enabled ({} req/s, burst {})",
            self.rate_limiter.config.requests_per_second,
            self.rate_limiter.config.burst_size);

        // Build axum app with CORS support
        let app = axum::Router::new()
            .route("/", axum::routing::post(handle_jsonrpc))
            .route("/__health", axum::routing::get(health_check))
            .layer(CorsLayer::permissive())
            .with_state((self.router, self.auth_config, self.rate_limiter));

        // Bind to address
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| MadrpcError::Transport(format!("Failed to bind to {}: {}", addr, e)))?;

        info!("Orchestrator HTTP server listening on {}", listener.local_addr()
            .map_err(|e| MadrpcError::Transport(format!("Failed to get local addr: {}", e)))?);

        // Run server
        axum::serve(listener, app)
            .await
            .map_err(|e| MadrpcError::Transport(format!("Server error: {}", e)))?;

        Ok(())
    }
}

/// Handles JSON-RPC POST requests.
///
/// # Arguments
/// * `State((router, auth_config, rate_limiter))` - Orchestrator router, auth config, and rate limiter
/// * `ConnectInfo(remote_addr)` - Client's socket address
/// * `headers` - Request headers (for Content-Type validation and auth)
/// * `body` - Request body bytes
///
/// # Returns
/// A JSON-RPC response or an HTTP error
async fn handle_jsonrpc(
    State((router, auth_config, rate_limiter)): State<(Arc<OrchestratorRouter>, AuthConfig, Arc<RateLimiter>)>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Check rate limit first (before expensive operations)
    let client_ip = remote_addr.ip();
    let rate_limit_result = rate_limiter.check_rate_limit(&client_ip).await;
    if !rate_limit_result.is_allowed() {
        tracing::warn!("Rate limit exceeded for IP: {}", client_ip);
        return rate_limited_response(rate_limit_result.retry_after());
    }

    // Check authentication if enabled
    if auth_config.requires_auth() {
        let api_key = headers.get("x-api-key")
            .and_then(|v| v.to_str().ok());

        if !auth_config.validate_api_key(api_key.unwrap_or("")) {
            tracing::warn!("Authentication failed: invalid or missing API key");
            return unauthorized_response();
        }
    }

    // Validate Content-Type
    if let Some(content_type) = headers.get("content-type") {
        if !content_type.to_str().unwrap_or("").contains("application/json") {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json",
            ).into_response();
        }
    }

    // Parse JSON-RPC request
    let jsonrpc_req = match HttpTransport::parse_jsonrpc(body) {
        Ok(req) => req,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid JSON-RPC: {}", e),
            ).into_response();
        }
    };

    // Handle request via router and convert to HTTP response
    let jsonrpc_response = router.handle_request(jsonrpc_req).await;

    // Convert to JSON bytes and create response
    let json_bytes = serde_json::to_vec(&jsonrpc_response).unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(json_bytes))
        .unwrap()
}

/// Handles health check GET requests.
///
/// # Returns
/// A simple "OK" response
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Creates an HTTP 401 Unauthorized response.
///
/// # Returns
/// A response with 401 status and appropriate error message
fn unauthorized_response() -> Response {
    let body = r#"{"jsonrpc":"2.0","error":{"code":-401,"message":"Unauthorized: Invalid or missing API key"},"id":null}"#;
    (StatusCode::UNAUTHORIZED, body).into_response()
}

/// Creates an HTTP 429 Too Many Requests response.
///
/// # Arguments
/// * `retry_after` - Optional duration until the request can be retried
///
/// # Returns
/// A response with 429 status and appropriate error message
fn rate_limited_response(retry_after: Option<std::time::Duration>) -> Response {
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

    (
        StatusCode::TOO_MANY_REQUESTS,
        [("Retry-After", &retry_after_secs)],
        body.to_string(),
    ).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health_checker::HealthCheckConfig;
    use crate::orchestrator::RetryConfig;

    #[tokio::test]
    async fn test_http_server_creation() {
        let orchestrator = Arc::new(
            Orchestrator::with_retry_config(
                vec![],
                HealthCheckConfig::default(),
                RetryConfig::default(),
            )
            .unwrap(),
        );

        let server = HttpServer::new(orchestrator);
        assert!(Arc::strong_count(&server.router) >= 1);

        // Verify authentication is disabled by default
        assert!(!server.auth_config.requires_auth());
    }

    #[tokio::test]
    async fn test_http_server_with_auth() {
        let orchestrator = Arc::new(
            Orchestrator::with_retry_config(
                vec![],
                HealthCheckConfig::default(),
                RetryConfig::default(),
            )
            .unwrap(),
        );

        let server = HttpServer::new(orchestrator)
            .with_auth(AuthConfig::with_api_key("test-key"));

        // Verify authentication is enabled
        assert!(server.auth_config.requires_auth());
        assert!(server.auth_config.validate_api_key("test-key"));
        assert!(!server.auth_config.validate_api_key("wrong-key"));
    }

    #[tokio::test]
    async fn test_health_check() {
        let response = health_check().await;
        let response = response.into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
