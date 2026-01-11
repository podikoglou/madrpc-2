//! HTTP Node Integration Tests
//!
//! This module tests the HTTP server implementation for the MaDRPC node.
//! Tests cover:
//! - Built-in methods (_metrics, _info)
//! - JavaScript method execution
//! - Error handling (method not found, invalid JSON, etc.)
//! - Custom headers support
//!
//! Tests use testcontainers for proper isolation.

use madrpc_common::protocol::JsonRpcRequest;
use reqwest::Client;
use serde_json::json;

mod testcontainers;
use testcontainers::{server_test_script, empty_script, NodeContainer};

/// Helper to make a JSON-RPC request
async fn jsonrpc_request(url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let client = Client::new();
    let body = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: method.into(),
        params,
        id: json!(1),
    };

    let res = client
        .post(url)
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
    let node = NodeContainer::start(empty_script())
        .await
        .expect("Failed to start node container");

    let response = jsonrpc_request(node.url(), "_info", json!({})).await;

    assert_eq!(response["result"]["server_type"], "node");
    assert!(response["result"]["uptime_ms"].is_number());
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

#[tokio::test]
async fn test_single_node_builtin_metrics() {
    let node = NodeContainer::start(empty_script())
        .await
        .expect("Failed to start node container");

    let response = jsonrpc_request(node.url(), "_metrics", json!({})).await;

    assert!(response["result"]["total_requests"].is_number());
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

// ============================================================================
// JavaScript Method Execution Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_javascript_method() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let response = jsonrpc_request(node.url(), "echo", json!({"msg": "hello"})).await;

    assert_eq!(response["result"]["msg"], "hello");
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

#[tokio::test]
async fn test_single_node_compute_method() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let response = jsonrpc_request(node.url(), "compute", json!({"x": 7, "y": 6})).await;

    assert_eq!(response["result"]["result"], 42);
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

#[tokio::test]
async fn test_single_node_async_method() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let response = jsonrpc_request(node.url(), "async_func", json!({"value": 21})).await;

    assert_eq!(response["result"]["result"], 42);
    assert!(response["error"].is_null() || response["error"] == json!(null));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_method_not_found() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let response = jsonrpc_request(node.url(), "nonexistent", json!({})).await;

    assert_eq!(response["error"]["code"], -32601); // Method not found
    assert_eq!(response["error"]["message"], "Method not found");
}

#[tokio::test]
async fn test_single_node_invalid_json() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let client = Client::new();
    let res = client
        .post(node.url())
        .body("invalid json")
        .send()
        .await
        .unwrap();

    let response: serde_json::Value = res.json().await.unwrap();

    assert_eq!(response["error"]["code"], -32700); // Parse error
}

#[tokio::test]
async fn test_single_node_invalid_params() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    // Try to compute without required parameters
    let response = jsonrpc_request(node.url(), "compute", json!({})).await;

    // The JavaScript will handle the error - should get an error response
    assert!(response["error"].is_object() || response["result"].is_null());
}

// ============================================================================
// Metrics Tracking Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_metrics_tracking() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    // Make some requests
    jsonrpc_request(node.url(), "echo", json!({"msg": "test1"})).await;
    jsonrpc_request(node.url(), "echo", json!({"msg": "test2"})).await;

    // Check metrics
    let response = jsonrpc_request(node.url(), "_metrics", json!({})).await;

    assert!(response["result"]["total_requests"].as_u64().unwrap() >= 2);
}

// ============================================================================
// Multiple Sequential Requests Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_sequential_requests() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    // Make multiple sequential requests
    for i in 0..5 {
        let response = jsonrpc_request(node.url(), "compute", json!({"x": i, "y": 2})).await;
        assert_eq!(response["result"]["result"], i * 2);
    }
}

// ============================================================================
// Concurrent Requests Tests
// ============================================================================

#[tokio::test]
async fn test_single_node_concurrent_requests() {
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let url = node.url().to_string();

    // Spawn multiple concurrent requests
    let mut handles = vec![];
    for i in 0..10 {
        let url_clone = url.clone();
        let handle = tokio::spawn(async move {
            let client = Client::new();
            let body = JsonRpcRequest {
                jsonrpc: "2.0".into(),
                method: "compute".into(),
                params: json!({"x": i, "y": 2}),
                id: json!(1),
            };

            let res = client
                .post(&url_clone)
                .json(&body)
                .send()
                .await
                .unwrap();

            res.json().await.unwrap()
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
    let node = NodeContainer::start(server_test_script())
        .await
        .expect("Failed to start node container");

    let client = Client::new();

    // Try GET request
    let res = client
        .get(node.url())
        .send()
        .await
        .unwrap();

    let response: serde_json::Value = res.json().await.unwrap();
    assert_eq!(response["error"]["code"], -32600); // Invalid request
}

// ============================================================================
// Request Size Limit Tests
// ============================================================================

#[tokio::test]
async fn test_request_size_limit_enforcement() {
    let node = NodeContainer::start(empty_script())
        .await
        .expect("Failed to start node container");

    let client = Client::new();

    // Create a request with a body that exceeds 10 MB
    let large_data = "x".repeat(11 * 1024 * 1024); // 11 MB
    let body = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: "_info".into(),
        params: json!({ "data": large_data }),
        id: json!(1),
    };

    let res = client
        .post(node.url())
        .json(&body)
        .send()
        .await
        .unwrap();

    let response: serde_json::Value = res.json().await.unwrap();
    assert_eq!(response["error"]["code"], -32001); // REQUEST_TOO_LARGE
    assert!(response["error"]["message"].as_str().unwrap().contains("too large"));
}

#[tokio::test]
async fn test_request_within_size_limit() {
    let node = NodeContainer::start(empty_script())
        .await
        .expect("Failed to start node container");

    let client = Client::new();

    // Create a request with a body that is well within the limit
    let body = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        method: "_info".into(),
        params: json!({}),
        id: json!(1),
    };

    let res = client
        .post(node.url())
        .json(&body)
        .send()
        .await
        .unwrap();

    let response: serde_json::Value = res.json().await.unwrap();
    // Should not get a REQUEST_TOO_LARGE error
    assert_ne!(response["error"]["code"], -32001);
}
