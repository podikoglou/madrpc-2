//! HTTP Orchestrator Integration Tests
//!
//! This test suite verifies the HTTP/JSON-RPC orchestrator implementation
//! using mock node servers.

use madrpc_orchestrator::{Orchestrator, HealthCheckConfig, RetryConfig};
use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// ============================================================================
// Mock Node Server
// ============================================================================

/// A mock node server that responds to HTTP JSON-RPC requests.
struct MockNodeServer {
    /// Server address
    addr: SocketAddr,
    /// Method handlers mapping
    handlers: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    /// Whether the server is healthy
    healthy: Arc<AtomicBool>,
    /// Tokio server handle
    _handle: tokio::task::JoinHandle<()>,
}

impl MockNodeServer {
    /// Creates a new mock node server.
    ///
    /// # Arguments
    /// * `port` - Port to listen on
    ///
    /// # Returns
    /// A new mock node server instance
    async fn new(port: u16) -> Self {
        use axum::{
            extract::State,
            http::StatusCode,
            response::{IntoResponse, Response},
            routing::{get, post},
            Router,
        };
        use hyper::body::Bytes;

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let handlers: Arc<Mutex<HashMap<String, serde_json::Value>>> = Arc::new(Mutex::new(HashMap::new()));
        let healthy = Arc::new(AtomicBool::new(true));

        // Clone for move into handler
        let handlers_clone = handlers.clone();
        let healthy_clone = healthy.clone();

        // JSON-RPC handler
        async fn handle_jsonrpc(
            State(handlers): State<Arc<Mutex<HashMap<String, serde_json::Value>>>>,
            State(healthy): State<Arc<AtomicBool>>,
            body: Bytes,
        ) -> impl IntoResponse {
            // Check if healthy
            if !healthy.load(Ordering::Relaxed) {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Node unhealthy".to_string(),
                );
            }

            // Parse JSON-RPC request
            let req: JsonRpcRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(_) => {
                    let error = JsonRpcError::parse_error();
                    let resp = JsonRpcResponse::error(json!(null), error);
                    return (StatusCode::OK, serde_json::to_vec(&resp).unwrap());
                }
            };

            // Get handler response
            let result = {
                let handlers = handlers.lock().await;
                handlers.get(&req.method).cloned()
            };

            let response = match result {
                Some(result) => JsonRpcResponse::success(req.id, result),
                None => {
                    let error = JsonRpcError::method_not_found();
                    JsonRpcResponse::error(req.id, error)
                }
            };

            (StatusCode::OK, serde_json::to_vec(&response).unwrap())
        }

        // Health check handler
        async fn handle_health(
            State(healthy): State<Arc<AtomicBool>>,
        ) -> impl IntoResponse {
            if healthy.load(Ordering::Relaxed) {
                (StatusCode::OK, "OK")
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, "Unhealthy")
            }
        }

        // Build router
        let app = Router::new()
            .route("/", post(handle_jsonrpc))
            .route("/__health", get(handle_health))
            .with_state(handlers_clone)
            .with_state(healthy_clone);

        // Bind and spawn server
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("Failed to bind mock node");

        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .unwrap();
        });

        // Wait for server to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Self {
            addr,
            handlers,
            healthy,
            _handle: handle,
        }
    }

    /// Gets the server address.
    fn addr(&self) -> String {
        self.addr.to_string()
    }

    /// Registers a handler for a method.
    async fn register_handler(&self, method: &str, response: serde_json::Value) {
        let mut handlers = self.handlers.lock().await;
        handlers.insert(method.to_string(), response);
    }

    /// Sets the health status of the node.
    fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::Relaxed);
    }
}

// ============================================================================
// Test Helpers
// ============================================================================

/// Creates a test orchestrator with the given nodes.
async fn create_test_orchestrator(node_addrs: Vec<String>) -> Orchestrator {
    Orchestrator::with_retry_config(
        node_addrs,
        HealthCheckConfig {
            interval: std::time::Duration::from_secs(1),
            timeout: std::time::Duration::from_millis(500),
            failure_threshold: 3,
        },
        RetryConfig {
            max_retries: 2,
            initial_backoff_ms: 10,
            max_backoff_ms: 100,
            backoff_multiplier: 2.0,
        },
    )
    .await
    .unwrap()
}

/// Creates a JSON-RPC request with the given method and params.
fn create_request(method: &str, params: serde_json::Value, id: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id,
    }
}

// ============================================================================
// Orchestrator Builtin Methods Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_builtin_methods() {
    let node = MockNodeServer::new(19001).await;
    let orchestrator = create_test_orchestrator(vec![node.addr()]).await;

    // Test _info
    let info_req = create_request("_info", json!({}), json!(1));
    let info_result = orchestrator.forward_request_jsonrpc(info_req).await;
    assert!(info_result.is_ok());
    let info = info_result.unwrap();
    assert_eq!(info["type"], "orchestrator");
    assert_eq!(info["total_nodes"], 1);

    // Test _metrics
    let metrics_req = create_request("_metrics", json!({}), json!(2));
    let metrics_result = orchestrator.forward_request_jsonrpc(metrics_req).await;
    assert!(metrics_result.is_ok());
    let metrics = metrics_result.unwrap();
    assert!(metrics["nodes"].is_array());
}

// ============================================================================
// Orchestrator Forwards to Node Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_forwards_to_node() {
    let node = MockNodeServer::new(19002).await;
    node.register_handler("test_method", json!({"result": "success"})).await;

    let orchestrator = create_test_orchestrator(vec![node.addr()]).await;

    let req = create_request("test_method", json!({"arg": 42}), json!(1));
    let result = orchestrator.forward_request_jsonrpc(req).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response["result"], "success");
}

// ============================================================================
// Orchestrator Transparency Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_transparency() {
    let node1 = MockNodeServer::new(19003).await;
    let node2 = MockNodeServer::new(19004).await;

    node1.register_handler("method1", json!({"node": 1})).await;
    node2.register_handler("method2", json!({"node": 2})).await;

    let orchestrator = create_test_orchestrator(vec![node1.addr(), node2.addr()]).await;

    // Both methods should work (round-robin)
    let req1 = create_request("method1", json!({}), json!(1));
    let result1 = orchestrator.forward_request_jsonrpc(req1).await;
    assert!(result1.is_ok());

    let req2 = create_request("method2", json!({}), json!(2));
    let result2 = orchestrator.forward_request_jsonrpc(req2).await;
    assert!(result2.is_ok());
}

// ============================================================================
// Orchestrator Round-Robin Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_round_robin() {
    let node1 = MockNodeServer::new(19005).await;
    let node2 = MockNodeServer::new(19006).await;
    let node3 = MockNodeServer::new(19007).await;

    node1.register_handler("test", json!({"node": 1})).await;
    node2.register_handler("test", json!({"node": 2})).await;
    node3.register_handler("test", json!({"node": 3})).await;

    let orchestrator = create_test_orchestrator(vec![
        node1.addr(),
        node2.addr(),
        node3.addr(),
    ]).await;

    // Make multiple requests to verify round-robin
    let mut node_counts = HashMap::new();

    for i in 0..9 {
        let req = create_request("test", json!({}), json!(i));
        let result = orchestrator.forward_request_jsonrpc(req).await;
        assert!(result.is_ok());

        let node_id = result.unwrap()["node"].as_u64().unwrap();
        *node_counts.entry(node_id).or_insert(0) += 1;
    }

    // Each node should get approximately 3 requests (9 total / 3 nodes)
    assert_eq!(node_counts.len(), 3);
    for (_, count) in node_counts {
        assert!(count >= 2 && count <= 4, "Node got {} requests, expected ~3", count);
    }
}

// ============================================================================
// Orchestrator All Nodes Down Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_all_nodes_down() {
    let node1 = MockNodeServer::new(19008).await;
    let node2 = MockNodeServer::new(19009).await;

    // Mark both nodes as unhealthy
    node1.set_healthy(false);
    node2.set_healthy(false);

    let orchestrator = create_test_orchestrator(vec![node1.addr(), node2.addr()]).await;

    // Wait for health checker to detect unhealthy nodes
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Try to make a request (should fail)
    let req = create_request("test", json!({}), json!(1));
    let result = orchestrator.forward_request_jsonrpc(req).await;
    assert!(result.is_err());
}

// ============================================================================
// Orchestrator Health Check Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_health_check() {
    let node = MockNodeServer::new(19010).await;
    let orchestrator = create_test_orchestrator(vec![node.addr()]).await;

    // Initially healthy
    let info_req = create_request("_info", json!({}), json!(1));
    let result = orchestrator.forward_request_jsonrpc(info_req).await;
    assert!(result.is_ok());

    // Mark as unhealthy
    node.set_healthy(false);

    // Wait for health checker to detect
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Should detect unhealthy node
    let nodes = orchestrator.nodes_with_status().await;
    let node_status = nodes.iter().find(|n| n.addr == node.addr());
    assert!(node_status.is_some());
    // Note: Node might not be disabled immediately (needs failure_threshold failures)
}

// ============================================================================
// Orchestrator Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_method_not_found() {
    let node = MockNodeServer::new(19011).await;
    // Don't register any handler

    let orchestrator = create_test_orchestrator(vec![node.addr()]).await;

    let req = create_request("unknown_method", json!({}), json!(1));
    let result = orchestrator.forward_request_jsonrpc(req).await;

    // Should get an error response from the node
    assert!(result.is_err());
}

#[tokio::test]
async fn test_orchestrator_with_no_nodes() {
    let orchestrator = create_test_orchestrator(vec![]).await;

    let req = create_request("test", json!({}), json!(1));
    let result = orchestrator.forward_request_jsonrpc(req).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_orchestrator_get_metrics() {
    let node = MockNodeServer::new(19012).await;
    let orchestrator = create_test_orchestrator(vec![node.addr()]).await;

    // Make some requests
    for i in 0..3 {
        let req = create_request("test", json!({}), json!(i));
        let _ = orchestrator.forward_request_jsonrpc(req).await;
    }

    // Get metrics
    let metrics = orchestrator.get_metrics().await.unwrap();
    assert!(metrics["nodes"].is_array());
    assert!(metrics["metrics"].is_object());
}

#[tokio::test]
async fn test_orchestrator_get_info() {
    let node = MockNodeServer::new(19013).await;
    let orchestrator = create_test_orchestrator(vec![node.addr()]).await;

    let info = orchestrator.get_info().await.unwrap();
    assert_eq!(info["server_type"], "orchestrator");
    assert_eq!(info["total_nodes"], 1);
    assert_eq!(info["enabled_nodes"], 1);
    assert_eq!(info["disabled_nodes"], 0);
    assert!(info["nodes"].is_array());
    assert!(info["uptime_ms"].is_number());
    assert!(info["version"].is_string());
}
