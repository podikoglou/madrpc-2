//! HTTP Server for Orchestrator
//!
//! This module provides the HTTP server implementation using axum.
//! It handles JSON-RPC requests and forwards them to the orchestrator router.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use hyper::body::Bytes;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::http_router::OrchestratorRouter;
use crate::orchestrator::Orchestrator;
use madrpc_common::protocol::error::MadrpcError;
use madrpc_common::transport::HttpTransport;

/// HTTP server for the orchestrator.
///
/// This server:
/// - Handles JSON-RPC POST requests at `/`
/// - Provides a health check endpoint at `/__health`
/// - Uses the orchestrator router for request routing
pub struct HttpServer {
    /// Orchestrator router for handling requests
    router: Arc<OrchestratorRouter>,
}

impl HttpServer {
    /// Creates a new HTTP server.
    ///
    /// # Arguments
    /// * `orchestrator` - Arc-wrapped orchestrator instance
    ///
    /// # Returns
    /// A new HTTP server instance
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        let router = Arc::new(OrchestratorRouter::new(orchestrator));
        Self { router }
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
        // Build axum app with CORS support
        let app = axum::Router::new()
            .route("/", axum::routing::post(handle_jsonrpc))
            .route("/__health", axum::routing::get(health_check))
            .layer(CorsLayer::permissive())
            .with_state(self.router);

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
/// * `State(router)` - Orchestrator router
/// * `headers` - Request headers (for Content-Type validation)
/// * `body` - Request body bytes
///
/// # Returns
/// A JSON-RPC response or an HTTP error
async fn handle_jsonrpc(
    State(router): State<Arc<OrchestratorRouter>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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
            .await
            .unwrap(),
        );

        let server = HttpServer::new(orchestrator);
        assert!(Arc::strong_count(&server.router) >= 1);
    }

    #[tokio::test]
    async fn test_health_check() {
        let response = health_check().await;
        let response = response.into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
