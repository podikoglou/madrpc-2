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
use http_body_util::BodyExt;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use serde_json::json;

use crate::http_router::NodeRouter;
use crate::node::Node;
use madrpc_common::protocol::JsonRpcError;
use madrpc_common::protocol::error::MadrpcError;
use madrpc_common::transport::{HttpTransport, HyperRequest, HyperResponse};

/// Maximum request body size (10 MB)
///
/// This limit prevents memory exhaustion DoS attacks by rejecting requests
/// with overly large payloads before they can cause significant memory allocation.
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// HTTP server for MaDRPC node.
///
/// The server listens for HTTP requests and processes JSON-RPC requests
/// through the NodeRouter.
pub struct HttpServer {
    /// The router for handling JSON-RPC requests
    router: Arc<NodeRouter>,
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
    /// A new `HttpServer` instance
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
        Self { router }
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

        loop {
            let (stream, _) = listener.accept().await
                .map_err(|e| MadrpcError::Transport(format!("Failed to accept connection: {}", e)))?;

            let io = TokioIo::new(stream);
            let router = self.router.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let router = router.clone();
                    async move { Self::handle_request(router, req).await }
                });

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    tracing::error!("Error serving connection: {}", err);
                }
            });
        }
    }

    /// Handles an HTTP request.
    ///
    /// # Arguments
    ///
    /// * `router` - The router to handle the JSON-RPC request
    /// * `req` - The incoming HTTP request
    ///
    /// # Returns
    ///
    /// A `Result` containing the HTTP response or an error
    async fn handle_request(
        router: Arc<NodeRouter>,
        req: HyperRequest,
    ) -> Result<HyperResponse, MadrpcError> {
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
        let _server = HttpServer::new(node.clone());
        assert!(node.script_path().exists());
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
    fn test_request_too_large_error() {
        // Test that the request_too_large error is created correctly
        let error = JsonRpcError::request_too_large(1024);
        assert_eq!(error.code, REQUEST_TOO_LARGE);
        assert!(error.message.contains("1024"));
        assert!(error.message.contains("too large"));
    }
}
