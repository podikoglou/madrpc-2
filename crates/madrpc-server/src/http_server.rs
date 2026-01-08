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
//!     let node = Arc::new(Node::new(script_path).unwrap());
//!     let server = HttpServer::new(node);
//!     server.run("127.0.0.1:8080".parse().unwrap()).await.unwrap();
//! }
//! ```

use hyper::{Request, Response, StatusCode};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::http_router::NodeRouter;
use crate::node::Node;
use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use madrpc_common::protocol::error::MadrpcError;
use madrpc_common::transport::{HttpTransport, HyperRequest, HyperResponse};

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
    /// let node = Arc::new(Node::new(script_path).unwrap());
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
    ///     let node = Arc::new(Node::new(script_path).unwrap());
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
        // Only accept POST requests
        if req.method() != hyper::Method::POST {
            return Ok(HttpTransport::to_http_error(
                json!(null),
                JsonRpcError::invalid_params("Only POST requests are supported")
            ));
        }

        // Read the request body
        let body = req.into_body().collect().await
            .map_err(|e| MadrpcError::Transport(format!("Failed to read request body: {}", e)))?
            .to_bytes();

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

        // Handle via ajj router
        let jsonrpc_res = match router.handle_request(jsonrpc_req).await {
            Ok(res) => res,
            Err(e) => {
                tracing::error!("Error handling request: {}", e);
                return Ok(HttpTransport::to_http_error(
                    json!(null),
                    JsonRpcError::internal_error(&e.to_string())
                ));
            }
        };

        Ok(HttpTransport::to_http_response(jsonrpc_res))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn create_test_script(content: &str) -> tempfile::NamedTempFile {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), content).unwrap();
        file
    }

    async fn start_test_server(node: Arc<Node>) -> SocketAddr {
        let server = HttpServer::new(node);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            // Transfer the listener to the server
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let router = server.router.clone();

            let service = service_fn(move |req| {
                let router = router.clone();
                async move { HttpServer::handle_request(router, req).await }
            });

            let _ = http1::Builder::new()
                .serve_connection(io, service)
                .await;
        });

        // Wait for server to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        addr
    }

    #[tokio::test]
    async fn test_server_creation() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let server = HttpServer::new(node);
        assert!(server.router.node.script_path().exists());
    }
}
