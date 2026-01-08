//! HTTP Router with ajj Integration
//!
//! This module provides the HTTP router for the MaDRPC node using the ajj
//! (async JSON-RPC) router library. The router handles built-in methods
//! (`_metrics`, `_info`) via explicit handlers and falls back to JavaScript
//! execution for all other methods.
//!
//! # Architecture
//!
//! The router uses a two-tier approach:
//! - **Built-in methods**: `_metrics` and `_info` are handled directly
//! - **JavaScript methods**: All other methods are forwarded to the Node for
//!   JavaScript execution via the fallback handler
//!
//! # Example
//!
//! ```no_run
//! use madrpc_server::http_router::NodeRouter;
//! use madrpc_server::Node;
//! use std::sync::Arc;
//!
//! let node = Arc::new(Node::new(script_path).unwrap());
//! let router = NodeRouter::new(node);
//! ```

use ajj::{Router, ResponsePayload};
use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use madrpc_common::protocol::error::MadrpcError;
use crate::node::Node;
use std::sync::Arc;
use serde_json::Value;

/// HTTP router for MaDRPC node with ajj integration.
///
/// The router handles built-in methods (`_metrics`, `_info`) and forwards
/// all other methods to JavaScript execution.
pub struct NodeRouter {
    /// The node instance for JavaScript execution
    node: Arc<Node>,
    /// The ajj router for handling JSON-RPC requests
    router: Router<Arc<Node>>,
}

impl NodeRouter {
    /// Creates a new node router with the given node instance.
    ///
    /// # Arguments
    ///
    /// * `node` - The node instance to use for JavaScript execution
    ///
    /// # Returns
    ///
    /// A new `NodeRouter` instance
    ///
    /// # Example
    ///
    /// ```no_run
    /// use madrpc_server::http_router::NodeRouter;
    /// use madrpc_server::Node;
    /// use std::sync::Arc;
    ///
    /// let node = Arc::new(Node::new(script_path).unwrap());
    /// let router = NodeRouter::new(node);
    /// ```
    pub fn new(node: Arc<Node>) -> Self {
        let router = Router::<Arc<Node>>::new()
            .route("_metrics", Self::metrics_handler)
            .route("_info", Self::info_handler)
            .fallback(|req: JsonRpcRequest, state: Arc<Node>| async move {
                Self::handle_javascript_method(state, req).await
            });

        Self { node, router }
    }

    /// Handles a JavaScript method call via the fallback handler.
    ///
    /// This method is called for all methods that are not built-in methods.
    /// It forwards the request to the Node for JavaScript execution.
    ///
    /// # Arguments
    ///
    /// * `node` - The node instance to execute JavaScript on
    /// * `req` - The JSON-RPC request
    ///
    /// # Returns
    ///
    /// A `Result` containing the result value or an ajj error
    async fn handle_javascript_method(
        node: Arc<Node>,
        req: JsonRpcRequest,
    ) -> Result<Value, ajj::Error> {
        match node.call_rpc(&req.method, req.params).await {
            Ok(result) => Ok(result),
            Err(e) => {
                // Convert MadrpcError to appropriate JSON-RPC error
                match e {
                    MadrpcError::InvalidRequest(msg) => {
                        Err(ajj::Error::from(JsonRpcError::invalid_params(&msg)))
                    }
                    _ => Err(ajj::Error::from(JsonRpcError::method_not_found())),
                }
            }
        }
    }

    /// Handles the `_metrics` built-in method.
    ///
    /// Returns a snapshot of current metrics including request counts,
    /// latency percentiles, and other statistics.
    ///
    /// # Arguments
    ///
    /// * `node` - The node instance to get metrics from
    /// * `_params` - The request parameters (ignored)
    ///
    /// # Returns
    ///
    /// A `Result` containing the metrics snapshot or an ajj error
    async fn metrics_handler(
        node: Arc<Node>,
        _params: Value,
    ) -> Result<Value, ajj::Error> {
        node.get_metrics()
            .await
            .map_err(|e| ajj::Error::server_error(&e.to_string()))
    }

    /// Handles the `_info` built-in method.
    ///
    /// Returns server information including server type, version, and uptime.
    ///
    /// # Arguments
    ///
    /// * `node` - The node instance to get info from
    /// * `_params` - The request parameters (ignored)
    ///
    /// # Returns
    ///
    /// A `Result` containing the server info or an ajj error
    async fn info_handler(
        node: Arc<Node>,
        _params: Value,
    ) -> Result<Value, ajj::Error> {
        node.get_info()
            .await
            .map_err(|e| ajj::Error::server_error(&e.to_string()))
    }

    /// Handles a JSON-RPC request through the router.
    ///
    /// # Arguments
    ///
    /// * `req` - The JSON-RPC request to handle
    ///
    /// # Returns
    ///
    /// A `JsonRpcResponse` with the result or error
    pub async fn handle_request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse, JsonRpcError> {
        self.router
            .handle_request_with_state(req, self.node.clone())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn create_test_script(content: &str) -> tempfile::NamedTempFile {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), content).unwrap();
        file
    }

    #[tokio::test]
    async fn test_router_creation() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let router = NodeRouter::new(node);
        assert!(router.node.script_path().exists());
    }

    #[tokio::test]
    async fn test_router_metrics_handler() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let result = NodeRouter::metrics_handler(node, json!({})).await;
        assert!(result.is_ok());
        let metrics = result.unwrap();
        assert!(metrics.is_object());
    }

    #[tokio::test]
    async fn test_router_info_handler() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let result = NodeRouter::info_handler(node, json!({})).await;
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.is_object());
    }
}
