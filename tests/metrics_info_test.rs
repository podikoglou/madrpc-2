//! Metrics Endpoints Integration Tests
//!
//! Comprehensive integration tests for the `_info` and `_metrics` endpoints
//! used by the `top` command for real-time monitoring.
//!
//! Test Scenarios:
//! 1. Single Node - _info and _metrics endpoints
//! 2. Orchestrator Only (no nodes) - _info and _metrics endpoints
//! 3. Orchestrator with Multiple Nodes - node distribution in metrics
//! 4. Orchestrator with RPC Calls - metrics tracking and latency
//! 5. Load Distribution Verification - request distribution across nodes
//! 6. Server Type Detection - discriminated union responses

use madrpc_client::MadrpcClient;
use serde_json::json;
use std::time::Duration;

mod testcontainers;
use testcontainers::{common_test_script, jsonrpc_call, OrchestratorContainer, NodeContainer};

// ============================================================================
// Scenario 1: Single Node - _info and _metrics Endpoints
// ============================================================================

#[tokio::test]
async fn test_single_node_info_endpoint() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Call _info endpoint
    let info_response = jsonrpc_call(node.url(), "_info", json!({}))
        .await
        .expect("_info call should succeed");

    let result = &info_response["result"];

    // Verify server type is "node"
    assert_eq!(result["server_type"], "node");

    // Verify version exists
    assert!(result["version"].is_string());
    assert!(!result["version"].as_str().unwrap().is_empty());

    // Verify uptime exists and is reasonable (> 0)
    let uptime_ms = result["uptime_ms"].as_u64().expect("uptime_ms should be a number");
    assert!(uptime_ms > 0, "Uptime should be greater than 0");
}

#[tokio::test]
async fn test_single_node_metrics_endpoint() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Call _metrics endpoint
    let metrics_response = jsonrpc_call(node.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];

    // Verify basic metrics structure
    assert!(result["total_requests"].is_number());
    assert!(result["successful_requests"].is_number());
    assert!(result["failed_requests"].is_number());
    assert!(result["active_connections"].is_number());
    assert!(result["uptime_ms"].is_number());

    // For a fresh node, request counts should be 0 or minimal (just the _metrics call)
    let total_requests = result["total_requests"].as_u64().unwrap();
    assert!(total_requests < 10, "Fresh node should have minimal requests");

    // Nodes should not have node distribution metrics
    assert!(result["nodes"].is_null() || result.get("nodes").is_none());
}

#[tokio::test]
async fn test_single_node_metrics_after_rpc() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Make a few RPC calls
    for i in 0..5 {
        client.call("echo", json!({"msg": format!("test_{}", i)})).await.unwrap();
    }

    // Get metrics
    let metrics_response = jsonrpc_call(node.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];

    // Should have at least 5 requests
    let total_requests = result["total_requests"].as_u64().unwrap();
    assert!(total_requests >= 5, "Should have at least 5 requests");

    // Should have successful requests
    let successful_requests = result["successful_requests"].as_u64().unwrap();
    assert!(successful_requests >= 5, "Should have at least 5 successful requests");

    // Should have method metrics for "echo"
    let methods = &result["methods"];
    assert!(methods.is_object(), "methods should be an object");
    assert!(methods.get("echo").is_some(), "Should have echo method metrics");

    // Verify echo method metrics
    let echo_metrics = &methods["echo"];
    assert_eq!(echo_metrics["call_count"], 5);
    assert_eq!(echo_metrics["success_count"], 5);
    assert_eq!(echo_metrics["failure_count"], 0);
}

// ============================================================================
// Scenario 2: Orchestrator Only (No Nodes)
// ============================================================================

#[tokio::test]
async fn test_orchestrator_only_info_endpoint() {
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator container");

    // Call _info endpoint
    let info_response = jsonrpc_call(orch.url(), "_info", json!({}))
        .await
        .expect("_info call should succeed");

    let result = &info_response["result"];

    // Verify server type is "orchestrator"
    assert_eq!(result["server_type"], "orchestrator");

    // Verify version exists
    assert!(result["version"].is_string());
    assert!(!result["version"].as_str().unwrap().is_empty());

    // Verify uptime exists
    let uptime_ms = result["uptime_ms"].as_u64().expect("uptime_ms should be a number");
    assert!(uptime_ms > 0, "Uptime should be greater than 0");

    // Verify node counts (should all be 0 for empty orchestrator)
    let total_nodes = result["total_nodes"].as_u64().unwrap();
    let enabled_nodes = result["enabled_nodes"].as_u64().unwrap();
    let disabled_nodes = result["disabled_nodes"].as_u64().unwrap();

    assert_eq!(total_nodes, 0, "Should have 0 total nodes");
    assert_eq!(enabled_nodes, 0, "Should have 0 enabled nodes");
    assert_eq!(disabled_nodes, 0, "Should have 0 disabled nodes");

    // nodes array should be empty
    let nodes = result["nodes"].as_array().expect("nodes should be an array");
    assert_eq!(nodes.len(), 0, "nodes array should be empty");
}

#[tokio::test]
async fn test_orchestrator_only_metrics_endpoint() {
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator container");

    // Call _metrics endpoint
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];

    // Verify basic metrics structure
    assert!(result["total_requests"].is_number());
    assert!(result["successful_requests"].is_number());
    assert!(result["failed_requests"].is_number());
    assert!(result["active_connections"].is_number());
    assert!(result["uptime_ms"].is_number());

    // Orchestrators should have nodes map (even if empty)
    let nodes = &result["nodes"];
    assert!(nodes.is_object(), "nodes should be an object/map");

    // Should be empty for orchestrator with no nodes
    if let serde_json::Value::Object(map) = nodes {
        assert_eq!(map.len(), 0, "nodes map should be empty");
    } else {
        panic!("nodes should be an object");
    }
}

// ============================================================================
// Scenario 3: Orchestrator with Multiple Nodes
// ============================================================================

#[tokio::test]
async fn test_orchestrator_multiple_nodes_info_endpoint() {
    // Start multiple nodes
    let mut nodes = vec![];
    for _ in 0..3 {
        let node = NodeContainer::start(common_test_script())
            .await
            .expect("Failed to start node container");
        nodes.push(node);
    }

    // Collect container URLs
    let node_addrs: Vec<String> = nodes.iter().map(|n| n.container_url.clone()).collect();

    // Start orchestrator with nodes
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    // Call _info endpoint
    let info_response = jsonrpc_call(orch.url(), "_info", json!({}))
        .await
        .expect("_info call should succeed");

    let result = &info_response["result"];

    // Verify server type
    assert_eq!(result["server_type"], "orchestrator");

    // Verify node counts
    let total_nodes = result["total_nodes"].as_u64().unwrap();
    let enabled_nodes = result["enabled_nodes"].as_u64().unwrap();
    let disabled_nodes = result["disabled_nodes"].as_u64().unwrap();

    assert_eq!(total_nodes, 3, "Should have 3 total nodes");
    assert_eq!(enabled_nodes, 3, "Should have 3 enabled nodes");
    assert_eq!(disabled_nodes, 0, "Should have 0 disabled nodes");

    // Verify nodes array has 3 entries
    let nodes_array = result["nodes"].as_array().expect("nodes should be an array");
    assert_eq!(nodes_array.len(), 3, "nodes array should have 3 entries");

    // Verify each node entry has required fields
    for node_info in nodes_array {
        assert!(node_info["addr"].is_string(), "Node should have addr");
        assert!(node_info["enabled"].is_boolean(), "Node should have enabled flag");
        assert_eq!(node_info["enabled"], true, "Nodes should be enabled");
        assert!(node_info["circuit_state"].is_string(), "Node should have circuit_state");
        assert!(node_info["consecutive_failures"].is_number(), "Node should have consecutive_failures");
    }
}

#[tokio::test]
async fn test_orchestrator_multiple_nodes_metrics_endpoint() {
    // Start multiple nodes
    let mut nodes = vec![];
    for _ in 0..3 {
        let node = NodeContainer::start(common_test_script())
            .await
            .expect("Failed to start node container");
        nodes.push(node);
    }

    // Collect container URLs
    let node_addrs: Vec<String> = nodes.iter().map(|n| n.container_url.clone()).collect();

    // Start orchestrator with nodes
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    // Call _metrics endpoint
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];

    // Orchestrators should have nodes map with entries
    let nodes_map = &result["nodes"];
    assert!(nodes_map.is_object(), "nodes should be an object/map");

    // Should have 3 nodes
    if let serde_json::Value::Object(map) = nodes_map {
        assert_eq!(map.len(), 3, "nodes map should have 3 entries");
    } else {
        panic!("nodes should be an object");
    }
}

// ============================================================================
// Scenario 4: Metrics Tracking After RPC Calls
// ============================================================================

#[tokio::test]
async fn test_single_node_latency_metrics() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Make multiple RPC calls to build latency data
    for _ in 0..10 {
        client.call("echo", json!({"msg": "latency_test"})).await.unwrap();
    }

    // Get metrics
    let metrics_response = jsonrpc_call(node.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];
    let echo_metrics = &result["methods"]["echo"];

    // Verify call counts
    assert_eq!(echo_metrics["call_count"], 10);
    assert_eq!(echo_metrics["success_count"], 10);
    assert_eq!(echo_metrics["failure_count"], 0);

    // Verify latency metrics exist
    assert!(echo_metrics["avg_latency_us"].is_number());
    assert!(echo_metrics["p50_latency_us"].is_number());
    assert!(echo_metrics["p95_latency_us"].is_number());
    assert!(echo_metrics["p99_latency_us"].is_number());

    // Latencies should be positive
    let avg_latency = echo_metrics["avg_latency_us"].as_u64().unwrap();
    assert!(avg_latency > 0, "Average latency should be greater than 0");

    let p50 = echo_metrics["p50_latency_us"].as_u64().unwrap();
    let p95 = echo_metrics["p95_latency_us"].as_u64().unwrap();
    let p99 = echo_metrics["p99_latency_us"].as_u64().unwrap();

    // Percentiles should be in ascending order (or equal)
    assert!(p50 <= p95, "P50 should be <= P95");
    assert!(p95 <= p99, "P95 should be <= P99");
}

#[tokio::test]
async fn test_orchestrator_multiple_methods_metrics() {
    // Start nodes
    let mut nodes = vec![];
    for _ in 0..2 {
        let node = NodeContainer::start(common_test_script())
            .await
            .expect("Failed to start node container");
        nodes.push(node);
    }

    let node_addrs: Vec<String> = nodes.iter().map(|n| n.container_url.clone()).collect();
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    let client = MadrpcClient::new(orch.url()).unwrap();

    // Call different methods
    for i in 0..5 {
        client.call("echo", json!({"msg": i})).await.unwrap();
    }

    for _ in 0..3 {
        client.call("add", json!({"a": 1, "b": 2})).await.unwrap();
    }

    for _ in 0..2 {
        client.call("compute", json!({"x": 2, "y": 3})).await.unwrap();
    }

    // Get metrics from orchestrator
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];
    let methods = &result["methods"];

    // Should have metrics for all three methods
    assert!(methods.get("echo").is_some(), "Should have echo metrics");
    assert!(methods.get("add").is_some(), "Should have add metrics");
    assert!(methods.get("compute").is_some(), "Should have compute metrics");

    // Verify call counts
    assert_eq!(methods["echo"]["call_count"], 5);
    assert_eq!(methods["echo"]["success_count"], 5);

    assert_eq!(methods["add"]["call_count"], 3);
    assert_eq!(methods["add"]["success_count"], 3);

    assert_eq!(methods["compute"]["call_count"], 2);
    assert_eq!(methods["compute"]["success_count"], 2);

    // Verify total requests
    let total_requests = result["total_requests"].as_u64().unwrap();
    assert!(total_requests >= 10, "Should have at least 10 total requests");
}

// ============================================================================
// Scenario 5: Load Distribution Across Nodes
// ============================================================================

#[tokio::test]
async fn test_orchestrator_load_distribution() {
    // Start multiple nodes
    let mut nodes = vec![];
    for _ in 0..3 {
        let node = NodeContainer::start(common_test_script())
            .await
            .expect("Failed to start node container");
        nodes.push(node);
    }

    let node_addrs: Vec<String> = nodes.iter().map(|n| n.container_url.clone()).collect();
    let orch = OrchestratorContainer::start(node_addrs)
        .await
        .expect("Failed to start orchestrator container");

    let client = MadrpcClient::new(orch.url()).unwrap();

    // Make many requests to ensure distribution
    for i in 0..30 {
        client.call("echo", json!({"req": i})).await.unwrap();
    }

    // Get metrics from orchestrator
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];

    // Verify node distribution
    let nodes_map = &result["nodes"];
    if let serde_json::Value::Object(map) = nodes_map {
        assert_eq!(map.len(), 3, "Should have 3 nodes");

        // Each node should have received some requests
        for (addr, node_metrics) in map {
            assert!(node_metrics["node_addr"].is_string());
            assert_eq!(node_metrics["node_addr"], *addr);

            let request_count = node_metrics["request_count"].as_u64().unwrap();
            assert!(request_count > 0, "Each node should have received requests: {} has {}", addr, request_count);

            // last_request_ms should be set (non-zero for recent requests)
            let last_request = node_metrics["last_request_ms"].as_u64().unwrap();
            assert!(last_request > 0, "Node {} should have last_request_ms set", addr);
        }
    } else {
        panic!("nodes should be an object");
    }

    // Total requests should be at least 30
    let total_requests = result["total_requests"].as_u64().unwrap();
    assert!(total_requests >= 30, "Should have at least 30 total requests");
}

// ============================================================================
// Scenario 6: Server Type Detection - Discriminated Union
// ============================================================================

#[tokio::test]
async fn test_server_type_discriminated_response() {
    // Test node response
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let node_info = jsonrpc_call(node.url(), "_info", json!({}))
        .await
        .expect("_info call should succeed");

    // Node response should have server_type: "node"
    assert_eq!(node_info["result"]["server_type"], "node");

    // Test orchestrator response
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator container");

    let orch_info = jsonrpc_call(orch.url(), "_info", json!({}))
        .await
        .expect("_info call should succeed");

    // Orchestrator response should have server_type: "orchestrator"
    assert_eq!(orch_info["result"]["server_type"], "orchestrator");
}

// ============================================================================
// Scenario 7: Failed Requests Metrics
// ============================================================================

#[tokio::test]
async fn test_failed_request_metrics() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Make some successful requests
    for _ in 0..3 {
        client.call("echo", json!({"msg": "success"})).await.unwrap();
    }

    // Make a request that will fail (missing required params)
    let _ = client.call("add", json!({"a": 5})).await; // Missing "b"

    // Get metrics
    let metrics_response = jsonrpc_call(node.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    let result = &metrics_response["result"];

    // Should have failed requests
    let failed_requests = result["failed_requests"].as_u64().unwrap();
    assert!(failed_requests > 0, "Should have at least one failed request");

    // total_requests should be successful + failed
    let total_requests = result["total_requests"].as_u64().unwrap();
    let successful_requests = result["successful_requests"].as_u64().unwrap();

    assert_eq!(
        total_requests,
        successful_requests + failed_requests,
        "Total should equal successful + failed"
    );

    // The "add" method should have failure_count
    if let Some(add_metrics) = result["methods"].get("add") {
        let failure_count = add_metrics["failure_count"].as_u64().unwrap_or(0);
        assert!(failure_count > 0, "add method should have failures");
    }
}

// ============================================================================
// Scenario 8: Metrics Snapshot Consistency
// ============================================================================

#[tokio::test]
async fn test_metrics_snapshot_consistency() {
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    let client = MadrpcClient::new(node.url()).unwrap();

    // Make some requests
    for i in 0..5 {
        client.call("echo", json!({"msg": i})).await.unwrap();
    }

    // Get multiple snapshots and verify consistency
    let snapshot1 = jsonrpc_call(node.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let snapshot2 = jsonrpc_call(node.url(), "_metrics", json!({}))
        .await
        .expect("_metrics call should succeed");

    // Uptime should increase
    let uptime1 = snapshot1["result"]["uptime_ms"].as_u64().unwrap();
    let uptime2 = snapshot2["result"]["uptime_ms"].as_u64().unwrap();
    assert!(uptime2 > uptime1, "Uptime should increase between snapshots");

    // Version should be the same
    let version1 = snapshot1["result"]["version"].as_str().unwrap();
    let version2 = snapshot2["result"]["version"].as_str().unwrap();
    assert_eq!(version1, version2, "Version should be consistent");
}
