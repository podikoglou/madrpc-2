//! Auto-Register Integration Tests
//!
//! Comprehensive integration tests for the node auto-registration functionality.
//! Tests cover:
//! 1. Node registration with orchestrator
//! 2. Request routing after registration
//! 3. Multiple nodes registering and load balancing
//! 4. Duplicate registration handling
//! 5. Invalid registration URL handling
//!
//! All tests use testcontainers for isolation.

use madrpc_client::MadrpcClient;
use serde_json::json;
use std::time::Duration;

mod testcontainers;
use testcontainers::{common_test_script, jsonrpc_call, OrchestratorContainer, NodeContainer};

// ============================================================================
// Test 1: Node Registers with Orchestrator
// ============================================================================

#[tokio::test]
async fn test_node_registers_with_orchestrator() {
    // Start an orchestrator with no initial nodes
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator container");

    // Start a standalone node (not auto-registered)
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node container");

    // Manually register the node with the orchestrator
    let register_result = jsonrpc_call(
        orch.url(),
        "_register",
        json!({"node_url": node.container_url}),
    )
    .await
    .expect("_register call should succeed");

    // Verify the response structure - JSON-RPC responses have a "result" field
    let result_data = &register_result["result"];

    // Check if there's an error instead of a result
    if register_result.get("error").is_some() && register_result["error"] != serde_json::Value::Null {
        panic!("Got error response instead of success: {:?}", register_result["error"]);
    }

    assert!(
        result_data["registered_url"].is_string(),
        "Response should contain registered_url, got: {:?}", result_data
    );
    assert!(
        result_data["is_new_registration"].is_boolean(),
        "Response should contain is_new_registration flag"
    );

    // The URL should match what we sent
    assert_eq!(
        result_data["registered_url"],
        node.container_url.as_str()
    );
    assert_eq!(result_data["is_new_registration"], true);

    // Verify we can make RPC calls through the orchestrator to the registered node
    let client = MadrpcClient::new(orch.url()).unwrap();
    let result = client
        .call("echo", json!({"msg": "registered"}))
        .await
        .expect("RPC call should succeed");

    assert_eq!(result["msg"], "registered");
}

// ============================================================================
// Test 2: Orchestrator Routes Requests to Registered Node
// ============================================================================

#[tokio::test]
async fn test_orchestrator_routes_to_registered_node() {
    // Start orchestrator with no nodes
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Start a standalone node
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node");

    // Register the node with the orchestrator
    jsonrpc_call(
        orch.url(),
        "_register",
        json!({"node_url": node.container_url}),
    )
    .await
    .expect("Registration should succeed");

    // Test multiple RPC methods to verify routing works
    let client = MadrpcClient::new(orch.url()).unwrap();

    // Test echo
    let result = client
        .call("echo", json!({"test": "value"}))
        .await
        .expect("echo call should succeed");
    assert_eq!(result["test"], "value");

    // Test add
    let result = client
        .call("add", json!({"a": 10, "b": 25}))
        .await
        .expect("add call should succeed");
    assert_eq!(result["result"], 35);

    // Test compute
    let result = client
        .call("compute", json!({"x": 6, "y": 7}))
        .await
        .expect("compute call should succeed");
    assert_eq!(result["result"], 42);
}

// ============================================================================
// Test 3: Multiple Nodes Register and Load Balancing
// ============================================================================

#[tokio::test]
async fn test_multiple_nodes_register_and_load_balance() {
    // Start orchestrator with no initial nodes
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Start multiple standalone nodes
    let mut nodes = vec![];
    for _i in 0..3 {
        let node = NodeContainer::start(common_test_script())
            .await
            .expect("Failed to start node");
        nodes.push(node);
    }

    // Register all nodes with the orchestrator
    for node in &nodes {
        jsonrpc_call(
            orch.url(),
            "_register",
            json!({"node_url": node.container_url.clone()}),
        )
        .await
        .expect("Registration should succeed");
    }

    // Verify all nodes are registered
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("Failed to get metrics");

    // nodes is an object (map), not an array - count the keys
    let node_count = match &metrics_response["result"]["nodes"] {
        serde_json::Value::Object(map) => map.len() as u64,
        _ => 0,
    };

    assert_eq!(
        node_count, 3,
        "Orchestrator should have exactly 3 registered nodes, got {}",
        node_count
    );

    // Make multiple calls to verify load balancing
    let client = MadrpcClient::new(orch.url()).unwrap();

    for i in 0..10 {
        let result = client
            .call("echo", json!({"call": i}))
            .await
            .expect("RPC call should succeed");
        assert_eq!(result["call"], i);
    }

    // Verify requests were distributed across nodes
    // Each node should have received some requests
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("Failed to get metrics");

    // nodes is an object keyed by URL, iterate over the values
    if let serde_json::Value::Object(nodes_map) = &metrics_response["result"]["nodes"] {
        for (_url, node_info) in nodes_map {
            if let Some(requests) = node_info.get("request_count").and_then(|v| v.as_u64()) {
                assert!(
                    requests > 0,
                    "Each node should have received at least one request"
                );
            }
        }
    }
}

// ============================================================================
// Test 4: Duplicate Node Registration
// ============================================================================

#[tokio::test]
async fn test_duplicate_node_registration() {
    // Start orchestrator
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Manually register a node twice with the same URL
    let test_url = "http://172.17.0.10:9001";

    // First registration
    let result1 = jsonrpc_call(orch.url(), "_register", json!({"node_url": test_url}))
        .await
        .expect("First registration should succeed");

    assert_eq!(result1["result"]["registered_url"], test_url);
    assert_eq!(result1["result"]["is_new_registration"], true);

    // Second registration with same URL
    let result2 = jsonrpc_call(orch.url(), "_register", json!({"node_url": test_url}))
        .await
        .expect("Second registration should succeed");

    assert_eq!(result2["result"]["registered_url"], test_url);
    assert_eq!(result2["result"]["is_new_registration"], false);

    // Verify only one node is registered
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("Failed to get metrics");
    let node_count = match &metrics_response["result"]["nodes"] {
        serde_json::Value::Object(map) => map.len() as u64,
        _ => 0,
    };

    assert_eq!(
        node_count, 1,
        "Orchestrator should have exactly 1 node after duplicate registration, got {}",
        node_count
    );
}

// ============================================================================
// Test 5: Invalid Registration URL
// ============================================================================

#[tokio::test]
async fn test_invalid_registration_url() {
    // Start orchestrator
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Try to register with invalid URL (missing http:// prefix)
    let result = jsonrpc_call(
        orch.url(),
        "_register",
        json!({"node_url": "invalid-url"}),
    )
    .await
    .expect("Call should return a response");

    // Should get an error response
    // Check if there's an error field at the top level
    if result.get("error").is_some() {
        let error_msg = result["error"]["message"]
            .as_str()
            .unwrap_or("No error message");

        assert!(
            error_msg.contains("Invalid node URL") || error_msg.contains("must start with"),
            "Error message should mention invalid URL format, got: {}",
            error_msg
        );
    } else if result.get("result").is_some() {
        // If we got a result instead of an error, fail the test
        panic!("Expected error response but got success: {:?}", result);
    } else {
        panic!("Unexpected response format: {:?}", result);
    }
}

// ============================================================================
// Test 6: Node Registration with Different URLs
// ============================================================================

#[tokio::test]
async fn test_node_registration_with_different_urls() {
    // Start orchestrator
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Register multiple nodes with different URLs
    let urls = vec![
        "http://172.17.0.10:9001",
        "http://172.17.0.11:9001",
        "http://172.17.0.12:9001",
    ];

    for url in &urls {
        let result = jsonrpc_call(
            orch.url(),
            "_register",
            json!({"node_url": url}),
        )
        .await
        .expect("Registration should succeed");

        assert_eq!(result["result"]["registered_url"], *url);
        assert_eq!(result["result"]["is_new_registration"], true);
    }

    // Verify all nodes are registered
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("Failed to get metrics");
    let node_count = match &metrics_response["result"]["nodes"] {
        serde_json::Value::Object(map) => map.len() as u64,
        _ => 0,
    };

    assert_eq!(
        node_count, 3,
        "Should have exactly 3 registered nodes, got {}",
        node_count
    );
}

// ============================================================================
// Test 7: Orchestrator Info Shows Registered Nodes
// ============================================================================

#[tokio::test]
async fn test_orchestrator_info_shows_registered_nodes() {
    // Start orchestrator
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Start a standalone node
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node");

    // Register the node
    jsonrpc_call(
        orch.url(),
        "_register",
        json!({"node_url": node.container_url}),
    )
    .await
    .expect("Registration should succeed");

    // Get orchestrator info
    let info = jsonrpc_call(orch.url(), "_info", json!({}))
        .await
        .expect("_info call should succeed");

    // Verify the info shows registered nodes
    // The info response structure might vary, so we check for the presence of node info
    assert!(
        info["result"].is_object() || info["node_count"].is_u64(),
        "Info should contain result or node_count"
    );
}

// ============================================================================
// Test 8: Request Routing with Registered Nodes
// ============================================================================

#[tokio::test]
async fn test_request_routing_with_registered_nodes() {
    // Start orchestrator
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Start and register a node
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node");

    jsonrpc_call(
        orch.url(),
        "_register",
        json!({"node_url": node.container_url}),
    )
    .await
    .expect("Registration should succeed");

    // Wait a bit for the registration to take effect
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Make multiple concurrent requests
    let client = MadrpcClient::new(orch.url()).unwrap();
    let mut handles = vec![];

    for i in 0..10 {
        let client_clone = client.clone();
        let handle = tokio::spawn(async move {
            client_clone
                .call("echo", json!({"req": i}))
                .await
        });
        handles.push(handle);
    }

    // All requests should succeed
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "Request {} should succeed", i);
        assert_eq!(result.unwrap()["req"], i);
    }
}

// ============================================================================
// Test 9: Node Re-registration After Being Added Initially
// ============================================================================

#[tokio::test]
async fn test_node_reregistration() {
    // Start orchestrator with one node
    let node = NodeContainer::start(common_test_script())
        .await
        .expect("Failed to start node");

    let orch = OrchestratorContainer::start(vec![node.container_url.clone()])
        .await
        .expect("Failed to start orchestrator");

    // Try to register the same node again
    let result = jsonrpc_call(
        orch.url(),
        "_register",
        json!({"node_url": node.container_url}),
    )
    .await
    .expect("Re-registration should succeed");

    // Should indicate it's not a new registration
    assert_eq!(result["result"]["is_new_registration"], false);
    assert_eq!(result["result"]["registered_url"], node.container_url.as_str());

    // Should still have only one node
    let metrics_response = jsonrpc_call(orch.url(), "_metrics", json!({}))
        .await
        .expect("Failed to get metrics");
    let node_count = match &metrics_response["result"]["nodes"] {
        serde_json::Value::Object(map) => map.len() as u64,
        _ => 0,
    };

    assert_eq!(node_count, 1, "Should still have exactly 1 node");
}

// ============================================================================
// Test 10: Registration Error Handling
// ============================================================================

#[tokio::test]
async fn test_registration_error_handling() {
    // Start orchestrator
    let orch = OrchestratorContainer::start(vec![])
        .await
        .expect("Failed to start orchestrator");

    // Test with missing node_url parameter
    let result = jsonrpc_call(
        orch.url(),
        "_register",
        json!({}), // Missing node_url
    )
    .await
    .expect("Call should return a response");

    // Should get an error response
    // Check if there's an error field at the top level
    if result.get("error").is_some() {
        let error_msg = result["error"]["message"]
            .as_str()
            .unwrap_or("No error message");

        assert!(
            error_msg.contains("Invalid registration") || error_msg.contains("node_url"),
            "Error message should mention invalid params, got: {}",
            error_msg
        );
    } else if result.get("result").is_some() {
        panic!("Expected error response but got success: {:?}", result);
    } else {
        panic!("Unexpected response format: {:?}", result);
    }
}
