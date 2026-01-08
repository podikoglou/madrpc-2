//! HTTP Node Integration Tests
//!
//! This module tests the HTTP server implementation for the MaDRPC node.
//! Tests cover:
//! - Built-in methods (_metrics, _info)
//! - JavaScript method execution
//! - Error handling (method not found, invalid JSON, etc.)
//! - Custom headers support

use madrpc_server::{HttpServer, Node};
use madrpc_common::protocol::JsonRpcRequest;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use std::fs;
use std::net::SocketAddr;

use reqwest::Client;
use serde_json::json;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Helper to create a test script with a registered function
fn create_test_script_with_register() -> tempfile::NamedTempFile {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let file = tempfile::NamedTempFile::new().unwrap();
    let content = r#"
        madrpc.register('echo', function(args) {
            return args;
        });

        madrpc.register('compute', function(args) {
            return { result: args.x * args.y };
        });

        madrpc.register('async_func', async function(args) {
            // Simulate async work
            return new Promise((resolve) => {
                setTimeout(() => {
                    resolve({ result: args.value * 2 });
                }, 10);
            });
        });
    "#;
    fs::write(file.path(), content).unwrap();
    file
}

/// Helper to create an empty test script
fn create_test_script_empty() -> tempfile::NamedTempFile {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), "// empty").unwrap();
    file
}

/// Helper to start a test server on a random port
async fn start_test_server(node: std::sync::Arc<Node>) -> SocketAddr {
    let server = HttpServer::new(node);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        server.run(addr).await.unwrap();
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    addr
}

/// Helper to make a JSON-RPC request
async fn jsonrpc_request(addr: &SocketAddr, method: &str, params: serde_json::Value) -> serde_json::Value {
    let client = Client::new();
    let body = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: method.into(),
        params,
        id: json!(1),
    };

    let res = client
        .post(format!("http://{}", addr))
        .json(&body)
        .send()
        .await
        .unwrap();

    res.json().await.unwrap()
}

// ============================================================================
// Built-in Methods Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_builtin_info() {
    let script = create_test_script_empty();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let response = jsonrpc_request(&addr, "_info", json!({})).await;

    assert_eq!(response["result"]["server_type"], "node");
    assert!(response["result"]["uptime_ms"].is_number());
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

#[tokio::test]
async fn test_single_node_builtin_metrics() {
    let script = create_test_script_empty();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let response = jsonrpc_request(&addr, "_metrics", json!({})).await;

    assert!(response["result"]["total_requests"].is_number());
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

// ============================================================================
// JavaScript Method Execution Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_javascript_method() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let response = jsonrpc_request(&addr, "echo", json!({"msg": "hello"})).await;

    assert_eq!(response["result"]["msg"], "hello");
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

#[tokio::test]
async fn test_single_node_compute_method() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let response = jsonrpc_request(&addr, "compute", json!({"x": 7, "y": 6})).await;

    assert_eq!(response["result"]["result"], 42);
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

#[tokio::test]
async fn test_single_node_async_method() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let response = jsonrpc_request(&addr, "async_func", json!({"value": 21})).await;

    assert_eq!(response["result"]["result"], 42);
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_method_not_found() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let response = jsonrpc_request(&addr, "nonexistent", json!({})).await;

    assert_eq!(response["error"]["code"], -32601); // Method not found
    assert_eq!(response["error"]["message"], "Method not found");
}

#[tokio::test]
async fn test_single_node_invalid_json() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let client = Client::new();
    let res = client
        .post(format!("http://{}", addr))
        .body("invalid json")
        .send()
        .await
        .unwrap();

    let response: serde_json::Value = res.json().await.unwrap();

    assert_eq!(response["error"]["code"], -32700); // Parse error
}

#[tokio::test]
async fn test_single_node_invalid_params() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    // Try to compute without required parameters
    let response = jsonrpc_request(&addr, "compute", json!({})).await;

    // The JavaScript will handle the error - should get an error response
    assert!(response["error"].is_object() || response["result"].is_null());
}

// ============================================================================
// Metrics Tracking Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_metrics_tracking() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    // Make some requests
    jsonrpc_request(&addr, "echo", json!({"msg": "test1"})).await;
    jsonrpc_request(&addr, "echo", json!({"msg": "test2"})).await;

    // Check metrics
    let response = jsonrpc_request(&addr, "_metrics", json!({})).await;

    assert!(response["result"]["total_requests"].as_u64().unwrap() >= 2);
}

// ============================================================================
// Multiple Sequential Requests Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_sequential_requests() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    // Make multiple sequential requests
    for i in 0..5 {
        let response = jsonrpc_request(&addr, "compute", json!({"x": i, "y": 2})).await;
        assert_eq!(response["result"]["result"], i * 2);
    }
}

// ============================================================================
// Concurrent Requests Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_concurrent_requests() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    // Spawn multiple concurrent requests
    let mut handles = vec![];
    for i in 0..10 {
        let addr_clone = addr;
        let handle = tokio::spawn(async move {
            jsonrpc_request(&addr_clone, "compute", json!({"x": i, "y": 2})).await
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    let mut results = vec![];
    for handle in handles {
        let response = handle.await.unwrap();
        results.push(response);
    }

    // Verify all requests succeeded
    assert_eq!(results.len(), 10);
    for (i, response) in results.iter().enumerate() {
        assert_eq!(response["result"]["result"], i as i64 * 2);
    }
}

// ============================================================================
// HTTP Method Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_only_post_allowed() {
    let script = create_test_script_with_register();
    let node = std::sync::Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let addr = start_test_server(node).await;

    let client = Client::new();

    // Try GET request
    let res = client
        .get(format!("http://{}", addr))
        .send()
        .await
        .unwrap();

    let response: serde_json::Value = res.json().await.unwrap();
    assert_eq!(response["error"]["code"], -32600); // Invalid request
}
