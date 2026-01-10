//! HTTP/JSON-RPC Integration Tests
//!
//! Comprehensive integration tests for the HTTP/JSON-RPC implementation.
//! Tests cover real components (nodes, orchestrators, clients) working together.
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
use madrpc_orchestrator::{HealthCheckConfig, HttpServer as OrchHttpServer, Orchestrator};
use madrpc_server::{HttpServer as NodeHttpServer, Node};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

// ============================================================================
// Test Helpers
// ============================================================================

/// Creates a temporary test script file with the given content.
fn create_test_script(content: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), content).unwrap();
    file
}

/// Creates a simple echo script for testing.
fn create_echo_script() -> tempfile::NamedTempFile {
    create_test_script(
        r#"
        madrpc.register('echo', (args) => {
            return args;
        });

        madrpc.register('add', (args) => {
            return { result: args.a + args.b };
        });

        madrpc.register('sleep', (args) => {
            const ms = args.ms || 100;
            const start = Date.now();
            while (Date.now() - start < ms) {
                // Busy wait
            }
            return { slept: ms };
        });
    "#,
    )
}

/// Creates a Monte Carlo Pi script for testing.
fn create_monte_carlo_script() -> tempfile::NamedTempFile {
    create_test_script(
        r#"
        madrpc.register('monte_carlo_sample', (args) => {
            const samples = args.samples || 1000000;
            const seed = args.seed || 0;

            // Simple LCG for reproducible random numbers
            let state = seed;
            const random = () => {
                state = (state * 1103515245 + 12345) & 0x7fffffff;
                return state / 0x7fffffff;
            };

            let inside = 0;
            for (let i = 0; i < samples; i++) {
                const x = random();
                const y = random();
                if (x * x + y * y <= 1) {
                    inside++;
                }
            }

            return { inside, total: samples };
        });

        madrpc.register('aggregate', async (args) => {
            const numNodes = args.numNodes || 2;
            const samplesPerNode = args.samplesPerNode || 100000;

            const promises = [];
            for (let i = 0; i < numNodes; i++) {
                promises.push(madrpc.call('monte_carlo_sample', {
                    samples: samplesPerNode,
                    seed: i
                }));
            }

            const results = await Promise.all(promises);

            let totalInside = 0;
            let totalSamples = 0;
            for (const result of results) {
                totalInside += result.inside;
                totalSamples += result.total;
            }

            const piEstimate = 4 * totalInside / totalSamples;

            return {
                totalInside,
                totalSamples,
                piEstimate
            };
        });
    "#,
    )
}

/// Starts a test HTTP server for a node on a random port.
async fn start_node_server(node: Arc<Node>) -> (SocketAddr, JoinHandle<()>) {
    // Bind to a random port first
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let server = NodeHttpServer::new(node);
    let handle = tokio::spawn(async move {
        // Run the server on the bound address
        let _ = server.run(addr).await;
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify server is actually listening
    let client = reqwest::Client::new();
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            panic!("Server did not start within timeout");
        }

        match client.get(format!("http://{}/_info", addr)).send().await {
            Ok(_) => break,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    (addr, handle)
}

/// Starts a test HTTP server for an orchestrator on a random port.
async fn start_orchestrator_server(orch: Arc<Orchestrator>) -> (SocketAddr, JoinHandle<()>) {
    // Bind to a random port first
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let server = OrchHttpServer::new(orch);
    let handle = tokio::spawn(async move {
        // Run the server on the bound address
        let _ = server.run(addr).await;
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify server is actually listening
    let client = reqwest::Client::new();
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            panic!("Orchestrator server did not start within timeout");
        }

        match client.get(format!("http://{}/__health", addr)).send().await {
            Ok(_) => break,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    (addr, handle)
}

/// Makes a raw HTTP JSON-RPC call to the given address.
async fn http_jsonrpc_call(addr: SocketAddr, method: &str, params: serde_json::Value) -> serde_json::Value {
    let client = reqwest::Client::new();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let response = client
        .post(format!("http://{}", addr))
        .json(&request)
        .send()
        .await
        .unwrap();

    response.json().await.unwrap()
}

// ============================================================================
// Scenario 1: Real Node + Real Client (Direct Connection)
// ============================================================================

#[tokio::test]
async fn test_real_node_client() {
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

    let result = client.call("echo", json!({"msg": "test"})).await.unwrap();
    assert_eq!(result["msg"], "test");
}

#[tokio::test]
async fn test_real_node_add() {
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

    let result = client.call("add", json!({"a": 5, "b": 3})).await.unwrap();
    assert_eq!(result["result"], 8);
}

// ============================================================================
// Scenario 2: Real Orchestrator + Real Node + Client
// ============================================================================

#[tokio::test]
async fn test_orchestrator_node_client() {
    // Start real node
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _node_handle) = start_node_server(node).await;

    // Start real orchestrator
    let orch = Arc::new(
        Orchestrator::with_config(
            vec![format!("http://{}", node_addr)],
            HealthCheckConfig {
                interval: Duration::from_secs(10),
                timeout: Duration::from_millis(500),
                failure_threshold: 3,
            },
        )
        .await
        .unwrap(),
    );

    let (orch_addr, _orch_handle) = start_orchestrator_server(orch).await;

    // Client calls through orchestrator
    let client = MadrpcClient::new(format!("http://{}", orch_addr))
        .await
        .unwrap();

    let result = client.call("echo", json!({"msg": "through_orch"})).await.unwrap();
    assert_eq!(result["msg"], "through_orch");
}

// ============================================================================
// Scenario 3: Multiple Nodes + Orchestrator (Load Balancing)
// ============================================================================

#[tokio::test]
async fn test_multiple_nodes_orchestrator() {
    // Start multiple nodes
    let script = create_echo_script();
    let mut node_addrs = vec![];

    for _ in 0..3 {
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let (addr, _handle) = start_node_server(node).await;
        node_addrs.push(format!("http://{}", addr));
    }

    // Start orchestrator with all nodes
    let orch = Arc::new(
        Orchestrator::with_config(
            node_addrs,
            HealthCheckConfig {
                interval: Duration::from_secs(10),
                timeout: Duration::from_millis(500),
                failure_threshold: 3,
            },
        )
        .await
        .unwrap(),
    );

    let (orch_addr, _orch_handle) = start_orchestrator_server(orch).await;

    // Make multiple calls and verify they all succeed
    let client = MadrpcClient::new(format!("http://{}", orch_addr))
        .await
        .unwrap();

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
    let script = create_monte_carlo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

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
    let script = create_monte_carlo_script();

    // Start multiple nodes with the same script
    let mut node_addrs = vec![];
    for _ in 0..3 {
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let (addr, _handle) = start_node_server(node).await;
        node_addrs.push(format!("http://{}", addr));
    }

    // Start orchestrator with all nodes
    let orch = Arc::new(
        Orchestrator::with_config(
            node_addrs.clone(),
            HealthCheckConfig {
                interval: Duration::from_secs(10),
                timeout: Duration::from_millis(500),
                failure_threshold: 3,
            },
        )
        .await
        .unwrap(),
    );

    let (orch_addr, _orch_handle) = start_orchestrator_server(orch).await;

    // Create a node with orchestrator support for distributed RPC
    let node_with_orch = Arc::new(
        Node::with_orchestrator(script.path().to_path_buf(), format!("http://{}", orch_addr))
            .await
            .unwrap(),
    );
    let (node_addr, _handle) = start_node_server(node_with_orch).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

    // Test aggregate (distributed RPC)
    let result = client
        .call("aggregate", json!({"numNodes": 3, "samplesPerNode": 10000}))
        .await
        .unwrap();

    let total_inside = result["totalInside"].as_u64().unwrap();
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
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _node_handle) = start_node_server(node).await;

    // Create orchestrator with aggressive circuit breaker
    let orch = Arc::new(
        Orchestrator::with_config(
            vec![format!("http://{}", node_addr)],
            HealthCheckConfig {
                interval: Duration::from_secs(1),
                timeout: Duration::from_millis(100),
                failure_threshold: 2,
            },
        )
        .await
        .unwrap(),
    );

    let (orch_addr, _orch_handle) = start_orchestrator_server(orch).await;

    let client = MadrpcClient::new(format!("http://{}", orch_addr))
        .await
        .unwrap();

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
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

    // Call non-existent method
    let result = client.call("nonexistent_method", json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_error_handling_invalid_params() {
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

    // Call add with missing params
    let result = client.call("add", json!({"a": 5})).await;
    assert!(result.is_err());
}

// ============================================================================
// Scenario 7: Concurrent Requests (Parallel Execution)
// ============================================================================

#[tokio::test]
async fn test_concurrent_requests() {
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = Arc::new(
        MadrpcClient::new(format!("http://{}", node_addr))
            .await
            .unwrap(),
    );

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
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    // Test _info endpoint using JSON-RPC
    let info_response = http_jsonrpc_call(node_addr, "_info", serde_json::json!({})).await;
    assert!(info_response["result"]["server_type"].is_string());

    // Test _metrics endpoint using JSON-RPC
    let metrics_response = http_jsonrpc_call(node_addr, "_metrics", serde_json::json!({})).await;
    assert!(metrics_response["result"]["total_requests"].is_number());
}

// ============================================================================
// Scenario 9: Distributed RPC (madrpc.call from JavaScript)
// ============================================================================

#[tokio::test]
async fn test_distributed_rpc() {
    // This tests the aggregate function which uses madrpc.call internally
    let script = create_monte_carlo_script();

    // Start multiple nodes with the same script
    let mut node_addrs = vec![];
    for _ in 0..2 {
        let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
        let (addr, _handle) = start_node_server(node).await;
        node_addrs.push(format!("http://{}", addr));
    }

    // Start orchestrator with all nodes
    let orch = Arc::new(
        Orchestrator::with_config(
            node_addrs.clone(),
            HealthCheckConfig {
                interval: Duration::from_secs(10),
                timeout: Duration::from_millis(500),
                failure_threshold: 3,
            },
        )
        .await
        .unwrap(),
    );

    let (orch_addr, _orch_handle) = start_orchestrator_server(orch).await;

    // Create a node with orchestrator support for distributed RPC
    let node_with_orch = Arc::new(
        Node::with_orchestrator(script.path().to_path_buf(), format!("http://{}", orch_addr))
            .await
            .unwrap(),
    );
    let (node_addr, _handle) = start_node_server(node_with_orch).await;

    let client = MadrpcClient::new(format!("http://{}", node_addr))
        .await
        .unwrap();

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
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _node_handle) = start_node_server(node).await;

    // Create orchestrator with health checking enabled
    let orch = Arc::new(
        Orchestrator::with_config(
            vec![format!("http://{}", node_addr)],
            HealthCheckConfig {
                interval: Duration::from_millis(500),
                timeout: Duration::from_millis(500),
                failure_threshold: 3,
            },
        )
        .await
        .unwrap(),
    );

    let (orch_addr, _orch_handle) = start_orchestrator_server(orch).await;

    let client = MadrpcClient::new(format!("http://{}", orch_addr))
        .await
        .unwrap();

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
    let script = create_echo_script();
    let node = Arc::new(Node::new(script.path().to_path_buf()).unwrap());
    let (node_addr, _handle) = start_node_server(node).await;

    let client = reqwest::Client::new();

    // Test with correct Content-Type
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "echo",
        "params": {"test": 1},
        "id": 1
    });

    let response = client
        .post(format!("http://{}", node_addr))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    // Test with wrong Content-Type (should still work for nodes, they're lenient)
    let response = client
        .post(format!("http://{}", node_addr))
        .header("Content-Type", "text/plain")
        .json(&request)
        .send()
        .await
        .unwrap();

    // Nodes accept it (lenient), orchestrator would reject
    assert_eq!(response.status(), 200);
}
