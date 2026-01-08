//! HTTP Router for MaDRPC Node
//!
//! This module provides the HTTP router for the MaDRPC node.
//! The router handles built-in methods (`_metrics`, `_info`) and forwards
//! all other methods to JavaScript execution.
//!
//! # Architecture
//!
//! The router uses a simple approach:
//! - **Built-in methods**: `_metrics` and `_info` are handled directly
//! - **JavaScript methods**: All other methods are forwarded to the Node for
//!   JavaScript execution
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

use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use madrpc_common::protocol::error::MadrpcError;
use crate::node::Node;
use std::sync::Arc;

/// HTTP router for MaDRPC node.
///
/// The router handles built-in methods (`_metrics`, `_info`) and forwards
/// all other methods to JavaScript execution.
pub struct NodeRouter {
    /// The node instance for JavaScript execution
    node: Arc<Node>,
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
        Self { node }
    }

    /// Handles a JSON-RPC request.
    ///
    /// This method routes the request to the appropriate handler:
    /// - `_metrics`: Returns metrics snapshot
    /// - `_info`: Returns server info
    /// - Other methods: Forwards to JavaScript execution
    ///
    /// # Arguments
    ///
    /// * `req` - The JSON-RPC request to handle
    ///
    /// # Returns
    ///
    /// A `Result` containing the JSON-RPC response or a JsonRpcError
    pub async fn handle_request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse, JsonRpcError> {
        let id = req.id.clone();

        // Handle built-in methods
        match req.method.as_str() {
            "_metrics" => {
                match self.node.get_metrics().await {
                    Ok(metrics) => Ok(JsonRpcResponse::success(id, metrics)),
                    Err(e) => Ok(JsonRpcResponse::error(id, JsonRpcError::internal_error(&e.to_string()))),
                }
            }
            "_info" => {
                match self.node.get_info().await {
                    Ok(info) => Ok(JsonRpcResponse::success(id, info)),
                    Err(e) => Ok(JsonRpcResponse::error(id, JsonRpcError::internal_error(&e.to_string()))),
                }
            }
            _ => {
                // Forward to JavaScript execution
                match self.node.call_rpc(&req.method, req.params).await {
                    Ok(result) => Ok(JsonRpcResponse::success(id, result)),
                    Err(e) => {
                        // Convert MadrpcError to appropriate JSON-RPC error
                        match e {
                            MadrpcError::InvalidRequest(msg) => {
                                Ok(JsonRpcResponse::error(id, JsonRpcError::invalid_params(&msg)))
                            }
                            MadrpcError::JavaScriptExecution(msg) => {
                                Ok(JsonRpcResponse::error(id, JsonRpcError::server_error(&msg)))
                            }
                            _ => Ok(JsonRpcResponse::error(id, JsonRpcError::method_not_found())),
                        }
                    }
                }
            }
        }
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
        let router = NodeRouter::new(node.clone());

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "_metrics".into(),
            params: json!({}),
            id: json!(1),
        };

        let response = router.handle_request(req).await.unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_router_info_handler() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let router = NodeRouter::new(node.clone());

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "_info".into(),
            params: json!({}),
            id: json!(1),
        };

        let response = router.handle_request(req).await.unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["server_type"], "node");
    }

    #[tokio::test]
    async fn test_router_javascript_handler() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let router = NodeRouter::new(node.clone());

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "echo".into(),
            params: json!({"msg": "hello"}),
            id: json!(1),
        };

        let response = router.handle_request(req).await.unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["msg"], "hello");
    }

    #[tokio::test]
    async fn test_router_method_not_found() {
        let script = create_test_script("// empty");
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let router = NodeRouter::new(node.clone());

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "nonexistent".into(),
            params: json!({}),
            id: json!(1),
        };

        let response = router.handle_request(req).await.unwrap();
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }
}
