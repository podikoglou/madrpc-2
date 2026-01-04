//! MaDRPC Integration Tests
//!
//! Comprehensive integration tests that validate the entire MaDRPC system works together.
//! These tests focus on components that are currently implemented:
//! - Protocol layer (Request/Response)
//! - Load balancer functionality
//! - Node request handling
//! - Orchestrator management
//!
//! Note: Full end-to-end QUIC networking tests are not included since QUIC servers
//! are not yet fully implemented.

use madrpc_common::protocol::{Request, Response};
use madrpc_server::Node;
use madrpc_orchestrator::{LoadBalancer, Orchestrator};
use serde_json::json;
use std::fs;
use tempfile::NamedTempFile;

/// Helper function to create temporary test scripts
fn create_test_script(content: &str) -> NamedTempFile {
    let file = NamedTempFile::new().unwrap();
    fs::write(file.path(), content).unwrap();
    file
}

/// Setup crypto provider for tests that use QUIC
fn setup_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

// ============================================================================
// Protocol Layer Tests
// ============================================================================

#[tokio::test]
async fn test_protocol_request_response_flow() {
    let request = Request::new("test_method", json!({"arg": 42}));
    let response = Response::success(request.id, json!({"result": "ok"}));

    assert_eq!(request.id, response.id);
    assert!(response.success);
    assert_eq!(response.result, Some(json!({"result": "ok"})));
    assert!(response.error.is_none());
}

#[tokio::test]
async fn test_protocol_request_construction() {
    let request = Request::new("compute", json!({"x": 10, "y": 20}));

    assert_eq!(request.method, "compute");
    assert_eq!(request.args, json!({"x": 10, "y": 20}));
    assert!(request.timeout_ms.is_none());
}

#[tokio::test]
async fn test_protocol_request_with_timeout() {
    let request = Request::new("test", json!({})).with_timeout(5000);

    assert_eq!(request.timeout_ms, Some(5000));
    assert_eq!(request.method, "test");
}

#[tokio::test]
async fn test_protocol_response_success() {
    let response = Response::success(123, json!({"data": "value"}));

    assert_eq!(response.id, 123);
    assert!(response.success);
    assert_eq!(response.result, Some(json!({"data": "value"})));
    assert!(response.error.is_none());
}

#[tokio::test]
async fn test_protocol_response_error() {
    let response = Response::error(456, "something went wrong");

    assert_eq!(response.id, 456);
    assert!(!response.success);
    assert_eq!(response.error, Some("something went wrong".to_string()));
    assert!(response.result.is_none());
}

#[tokio::test]
async fn test_protocol_response_serialization() {
    let response = Response::success(789, json!({"status": "ok"}));

    // Test that response can be serialized and deserialized
    let serialized = serde_json::to_string(&response).unwrap();
    let deserialized: Response = serde_json::from_str(&serialized).unwrap();

    assert_eq!(response, deserialized);
}

#[tokio::test]
async fn test_protocol_request_serialization() {
    let request = Request::new("method", json!({"key": "value"})).with_timeout(1000);

    let serialized = serde_json::to_string(&request).unwrap();
    let deserialized: Request = serde_json::from_str(&serialized).unwrap();

    assert_eq!(request.method, deserialized.method);
    assert_eq!(request.args, deserialized.args);
    assert_eq!(request.timeout_ms, deserialized.timeout_ms);
}

// ============================================================================
// Load Balancer Tests
// ============================================================================

#[tokio::test]
async fn test_load_balancer_round_robin() {
    let nodes = vec!["node1".to_string(), "node2".to_string(), "node3".to_string()];
    let mut lb = LoadBalancer::new(nodes);

    assert_eq!(lb.next_node(), Some("node1".to_string()));
    assert_eq!(lb.next_node(), Some("node2".to_string()));
    assert_eq!(lb.next_node(), Some("node3".to_string()));
    assert_eq!(lb.next_node(), Some("node1".to_string())); // wraps around
}

#[tokio::test]
async fn test_load_balancer_empty_returns_none() {
    let mut lb = LoadBalancer::new(vec![]);
    assert_eq!(lb.next_node(), None);
}

#[tokio::test]
async fn test_load_balancer_single_node() {
    let mut lb = LoadBalancer::new(vec!["only".to_string()]);
    assert_eq!(lb.next_node(), Some("only".to_string()));
    assert_eq!(lb.next_node(), Some("only".to_string()));
}

#[tokio::test]
async fn test_load_balancer_node_count() {
    let nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let lb = LoadBalancer::new(nodes);
    assert_eq!(lb.node_count(), 3);
}

#[tokio::test]
async fn test_load_balancer_add_node() {
    let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
    lb.add_node("node2".to_string());
    assert_eq!(lb.node_count(), 2);
    assert_eq!(lb.nodes(), vec!["node1".to_string(), "node2".to_string()]);
}

#[tokio::test]
async fn test_load_balancer_add_duplicate_node() {
    let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
    lb.add_node("node1".to_string()); // duplicate
    assert_eq!(lb.node_count(), 1);
}

#[tokio::test]
async fn test_load_balancer_remove_node() {
    let mut lb = LoadBalancer::new(vec![
        "node1".to_string(),
        "node2".to_string(),
        "node3".to_string(),
    ]);
    lb.remove_node("node2");
    assert_eq!(lb.node_count(), 2);
    assert_eq!(lb.nodes(), vec!["node1".to_string(), "node3".to_string()]);
}

#[tokio::test]
async fn test_load_balancer_get_nodes() {
    let nodes = vec!["first".to_string(), "second".to_string()];
    let lb = LoadBalancer::new(nodes.clone());
    assert_eq!(lb.nodes(), nodes);
}

// ============================================================================
// Orchestrator Tests
// ============================================================================

#[tokio::test]
async fn test_orchestrator_creation() {
    setup_crypto();
    let nodes = vec!["localhost:9001".to_string(), "localhost:9002".to_string()];
    let orch = Orchestrator::new(nodes).await;
    assert!(orch.is_ok());
}

#[tokio::test]
async fn test_orchestrator_node_count() {
    setup_crypto();
    let nodes = vec![
        "localhost:9001".to_string(),
        "localhost:9002".to_string(),
        "localhost:9003".to_string(),
    ];
    let orch = Orchestrator::new(nodes).await.unwrap();
    assert_eq!(orch.node_count().await, 3);
}

#[tokio::test]
async fn test_orchestrator_add_remove_nodes() {
    setup_crypto();
    let orch = Orchestrator::new(vec![]).await.unwrap();

    orch.add_node("node1".to_string()).await;
    assert_eq!(orch.node_count().await, 1);

    orch.add_node("node2".to_string()).await;
    assert_eq!(orch.node_count().await, 2);

    orch.remove_node("node1").await;
    assert_eq!(orch.node_count().await, 1);

    let nodes = orch.nodes().await;
    assert_eq!(nodes, vec!["node2".to_string()]);
}

#[tokio::test]
async fn test_orchestrator_get_nodes() {
    setup_crypto();
    let nodes = vec![
        "node1".to_string(),
        "node2".to_string(),
    ];
    let orch = Orchestrator::new(nodes.clone()).await.unwrap();
    assert_eq!(orch.nodes().await, nodes);
}

#[tokio::test]
async fn test_orchestrator_add_duplicate_node() {
    setup_crypto();
    let orch = Orchestrator::new(vec!["node1".to_string()]).await.unwrap();
    orch.add_node("node1".to_string()).await;
    assert_eq!(orch.node_count().await, 1);
}

#[tokio::test]
async fn test_orchestrator_empty() {
    setup_crypto();
    let orch = Orchestrator::new(vec![]).await.unwrap();
    assert_eq!(orch.node_count().await, 0);
    assert_eq!(orch.nodes().await, Vec::<String>::new());
}

// ============================================================================
// Node RPC Handling Tests
// ============================================================================

#[tokio::test]
async fn test_node_returns_error_for_invalid_method() {
    let script = create_test_script("// no functions");
    let node = Node::new(script.path().to_path_buf()).await.unwrap();

    let request = Request::new("nonexistent", json!({}));
    let response = node.handle_request(&request).await.unwrap();

    assert!(!response.success);
    assert!(response.error.is_some());
}

#[tokio::test]
async fn test_node_script_path() {
    let script = create_test_script("// test script");
    let node = Node::new(script.path().to_path_buf()).await.unwrap();

    assert_eq!(node.script_path(), script.path());
}

// ============================================================================
// End-to-End Flow Tests (Mock)
// ============================================================================

#[tokio::test]
async fn test_end_to_end_protocol_flow() {
    // Test the full protocol flow without actual Node execution
    let request = Request::new("test_method", json!({"key": "value"}));
    let request_id = request.id;

    // Simulate successful processing
    let response = Response::success(request_id, json!({"status": "ok", "data": "result"}));

    assert!(response.success);
    assert_eq!(response.id, request_id);
    assert_eq!(response.result, Some(json!({"status": "ok", "data": "result"})));
}

#[tokio::test]
async fn test_end_to_end_error_flow() {
    let request = Request::new("failing_method", json!({}));

    // Simulate error response
    let response = Response::error(request.id, "Method not found");

    assert!(!response.success);
    assert_eq!(response.id, request.id);
    assert_eq!(response.error, Some("Method not found".to_string()));
}

#[tokio::test]
async fn test_end_to_end_orchestrator_flow() {
    setup_crypto();

    // Create orchestrator with multiple nodes
    let nodes = vec![
        "node1.example.com".to_string(),
        "node2.example.com".to_string(),
        "node3.example.com".to_string(),
    ];

    let orch = Orchestrator::new(nodes).await.unwrap();

    // Verify node management
    assert_eq!(orch.node_count().await, 3);

    // Add a new node
    orch.add_node("node4.example.com".to_string()).await;
    assert_eq!(orch.node_count().await, 4);

    // Remove a node
    orch.remove_node("node2.example.com").await;
    assert_eq!(orch.node_count().await, 3);

    let remaining_nodes = orch.nodes().await;
    assert_eq!(remaining_nodes.len(), 3);
}

#[tokio::test]
async fn test_end_to_end_load_balancer_selection() {
    // Create load balancer with multiple nodes
    let nodes = vec![
        "server1".to_string(),
        "server2".to_string(),
        "server3".to_string(),
    ];

    let mut lb = LoadBalancer::new(nodes);

    // Simulate multiple requests and verify round-robin
    let mut selections = Vec::new();
    for _ in 0..9 {
        if let Some(node) = lb.next_node() {
            selections.push(node);
        }
    }

    // Should have selected each node 3 times
    assert_eq!(selections.len(), 9);
    assert_eq!(selections[0], "server1");
    assert_eq!(selections[1], "server2");
    assert_eq!(selections[2], "server3");
    assert_eq!(selections[3], "server1"); // wraps around
}
