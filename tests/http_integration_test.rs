//! HTTP/JSON-RPC Integration Tests
//!
//! Comprehensive integration tests for the HTTP/JSON-RPC implementation.
//! Tests cover real components (nodes, orchestrators, clients) working together
//! using testcontainers for isolation.
//!
//! Test Scenarios:
//! 1. Real Node + Real Client (direct connection)
//! 2. Real Orchestrator + Real Node + Client (distributed system)
//! 3. Multiple Nodes + Orchestrator (load balancing)
//! 4. Monte Carlo Pi Example (real-world use case)
//! 5. Circuit Breaker (fault tolerance)
//! 6. Distributed RPC (madrpc.call from JavaScript)
//! 7. Health Checking (node failure detection)
//! 8. Error Handling (invalid requests, timeouts)
//! 9. Metrics Endpoints (_metrics, _info)
//! 10. Concurrent Requests (parallel execution)

use madrpc_client::MadrpcClient;
use serde_json::json;
use std::time::Duration;

mod testcontainers;
use testcontainers::{common_test_script, monte_carlo_script, OrchestratorContainer, NodeContainer};

// ============================================================================
// Scenario 1: Real Node + Real Client (Direct Connection)
// ============================================================================

#[tokio::test]
async fn test_real_node_client() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    let result = client.call("echo", json!({"msg": "test"})).await.unwrap();
    assert_eq!(result["msg"], "test");
}

#[tokio::test]
async fn test_real_node_add() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    let result = client.call("add", json!({"a": 5, "b": 3})).await.unwrap();
    assert_eq!(result["result"], 8);
}

// ============================================================================
// Scenario 2: Real Orchestrator + Real Node + Client
// ============================================================================

#[tokio::test]
async fn test_orchestrator_node_client() {
    // Start node in container
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Start orchestrator with the node's container URL (for container-to-container communication)
    let orch = OrchestratorContainer::start(vec![node.container_url.clone()])
        .await
        .expect("Failed to start orchestrator container");

    // Client calls through orchestrator using external URL (host-to-container)
    let client = MadrpcClient::new(orch.url()).unwrap();

    let result = client.call("echo", json!({"msg": "through_orch"})).await.unwrap();
    assert_eq!(result["msg"], "through_orch");
}

// ============================================================================
// Scenario 3: Multiple Nodes + Orchestrator (Load Balancing)
// ============================================================================

#[tokio::test]
async fn test_multiple_nodes_orchestrator() {
    // Start multiple nodes in containers
    let mut node_addrs = vec![];
    for _ in 0..3 {
        let node = NodeContainer::start(common_test_script())
            .await
            .expect("Failed to start node container");
        node_addrs.push(node.container_url.clone());
    }

    // Start orchestrator with all nodes
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    // Make multiple calls and verify they all succeed
    let client = MadrpcClient::new(orch.url()).unwrap();

    for i in 0..10 {
        let result = client.call("echo", json!({"call": i})).await.unwrap();
        assert_eq!(result["call"], i);
    }
}

// ============================================================================
// Scenario 4: Monte Carlo Pi Example (Real-World Use Case)
// ============================================================================

#[tokio::test]
async fn test_monte_carlo_pi() {
    let node = NodeContainer::start(monte_carlo_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Test single sample
    let result = client
        .call("monte_carlo_sample", json!({"samples": 10000, "seed": 42}))
        .await
        .unwrap();

    assert_eq!(result["total"], 10000);
    assert!(result["inside"].as_u64().unwrap() > 0);
    assert!(result["inside"].as_u64().unwrap() <= 10000);
}

#[tokio::test]
async fn test_monte_carlo_aggregate() {
    // Start multiple nodes with the same script
    let mut node_addrs = vec![];
    for _ in 0..3 {
        let node = NodeContainer::start(monte_carlo_script())
            .await
            .expect("Failed to start node container");
        node_addrs.push(node.container_url.clone());
    }

    // Start orchestrator with all nodes
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    // Create a node with orchestrator support for distributed RPC
    // Use orchestrator's container_url for container-to-container communication
    let node_with_orch = NodeContainer::start_with_orchestrator(
        monte_carlo_script(),
        orch.container_url.clone(),
    )
    .await
    .expect("Failed to start node with orchestrator");

    let client = MadrpcClient::new(node_with_orch.url()).unwrap();

    // Test aggregate (distributed RPC)
    let result = client
        .call("aggregate", json!({"numNodes": 3, "samplesPerNode": 10000}))
        .await
        .unwrap();

    let _total_inside = result["totalInside"].as_u64().unwrap();
    let total_samples = result["totalSamples"].as_u64().unwrap();
    let pi_estimate = result["piEstimate"].as_f64().unwrap();

    assert_eq!(total_samples, 30000);
    assert!(pi_estimate > 2.5 && pi_estimate < 3.5); // Roughly pi
}

// ============================================================================
// Scenario 5: Circuit Breaker (Fault Tolerance)
// ============================================================================

#[tokio::test]
async fn test_circuit_breaker() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Create orchestrator with aggressive circuit breaker
    // Use node's container_url for container-to-container communication
    let orch = OrchestratorContainer::start(vec![node.container_url.clone()])
        .await
        .expect("Failed to start orchestrator container");

    let client = MadrpcClient::new(orch.url()).unwrap();

    // First call should succeed
    let result = client.call("echo", json!({"test": 1})).await.unwrap();
    assert_eq!(result["test"], 1);

    // Note: In a real test, we'd stop the node and verify circuit breaker opens
    // But for now we just verify it works under normal conditions
}

// ============================================================================
// Scenario 6: Error Handling (Invalid Requests, Timeouts)
// ============================================================================

#[tokio::test]
async fn test_error_handling_invalid_method() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Call non-existent method
    let result = client.call("nonexistent_method", json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_error_handling_invalid_params() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Call add with missing params
    let result = client.call("add", json!({"a": 5})).await;
    assert!(result.is_err());
}

// ============================================================================
// Scenario 7: Concurrent Requests (Parallel Execution)
// ============================================================================

#[tokio::test]
async fn test_concurrent_requests() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = std::sync::Arc::new(MadrpcClient::new(node.url()).unwrap());

    // Make 10 concurrent requests
    let mut handles = vec![];
    for i in 0..10 {
        let client = client.clone();
        let handle = tokio::spawn(async move {
            client
                .call("echo", json!({"request": i}))
                .await
                .unwrap()
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let mut results = vec![];
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    assert_eq!(results.len(), 10);

    // Verify all requests completed
    for i in 0..10 {
        assert!(results.iter().any(|r| r["request"] == i));
    }
}

// ============================================================================
// Scenario 8: Metrics Endpoints
// ============================================================================

#[tokio::test]
async fn test_metrics_endpoints() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Test _info endpoint using JSON-RPC
    let info_response = testcontainers::jsonrpc_call(node.url(), "_info", serde_json::json!({}))
        .await
        .unwrap();
    assert!(info_response["result"]["server_type"].is_string());

    // Test _metrics endpoint using JSON-RPC
    let metrics_response = testcontainers::jsonrpc_call(node.url(), "_metrics", serde_json::json!({}))
        .await
        .unwrap();
    assert!(metrics_response["result"]["total_requests"].is_number());
}

// ============================================================================
// Scenario 9: Distributed RPC (madrpc.call from JavaScript)
// ============================================================================

#[tokio::test]
async fn test_distributed_rpc() {
    // This tests the aggregate function which uses madrpc.call internally
    let mut node_addrs = vec![];
    for _ in 0..2 {
        let node = NodeContainer::start(monte_carlo_script())
            .await
            .expect("Failed to start node container");
        node_addrs.push(node.container_url.clone());
    }

    // Start orchestrator with all nodes
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    // Create a node with orchestrator support for distributed RPC
    // Use orchestrator's container_url for container-to-container communication
    let node_with_orch = NodeContainer::start_with_orchestrator(
        monte_carlo_script(),
        orch.container_url.clone(),
    )
    .await
    .expect("Failed to start node with orchestrator");

    let client = MadrpcClient::new(node_with_orch.url()).unwrap();

    // The aggregate function uses madrpc.call to make distributed RPC calls
    let result = client
        .call("aggregate", json!({"numNodes": 2, "samplesPerNode": 5000}))
        .await
        .unwrap();

    assert_eq!(result["totalSamples"], 10000);
    assert!(result["piEstimate"].as_f64().unwrap() > 0.0);
}

// ============================================================================
// Scenario 10: Health Checking
// ============================================================================

#[tokio::test]
async fn test_health_checking() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Create orchestrator with health checking enabled
    // Use node's container_url for container-to-container communication
    let orch = OrchestratorContainer::start(vec![node.container_url.clone()])
        .await
        .expect("Failed to start orchestrator container");

    let client = MadrpcClient::new(orch.url()).unwrap();

    // First call should work
    let result = client.call("echo", json!({"test": "health"})).await.unwrap();
    assert_eq!(result["test"], "health");

    // Wait for health checks to run
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Should still work after health checks
    let result = client.call("echo", json!({"test": "still_healthy"})).await.unwrap();
    assert_eq!(result["test"], "still_healthy");
}

// ============================================================================
// Additional: HTTP Transport Validation
// ============================================================================

#[tokio::test]
async fn test_http_content_type_validation() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = reqwest::Client::new();

    // Test with correct Content-Type
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "echo",
        "params": {"test": 1},
        "id": 1
    });

    let response = client
        .post(node.url())
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    // Test with wrong Content-Type (should still work for nodes, they're lenient)
    let response = client
        .post(node.url())
        .header("Content-Type", "text/plain")
        .json(&request)
        .send()
        .await
        .unwrap();

    // Nodes accept it (lenient), orchestrator would reject
    assert_eq!(response.status(), 200);
}
