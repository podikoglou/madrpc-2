// Integration tests for madrpc-server
//
// These tests create a real TCP server with a test JavaScript script,
// then connect a client to make RPC calls.

use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::Result as MadrpcResult;
use madrpc_common::transport::tcp_server::TcpServerThreaded;
use madrpc_server::Node;
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

// ============================================================================
// Test Helpers
// ============================================================================

/// Counter for generating unique test script names
static TEST_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Create a temporary test script with the given content
fn create_test_script(content: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = PathBuf::from(format!("/tmp/test_madrpc_integration_{}.js", id));
    fs::write(&path, content).expect("Failed to write test script");
    path
}

/// Simple TCP client that sends requests and receives responses
struct TestClient {
    stream: TcpStream,
}

impl TestClient {
    /// Connect to a server at the given address
    fn connect(addr: &str) -> MadrpcResult<Self> {
        let stream = TcpStream::connect(addr)
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to connect: {}", e),
            ))?;
        Ok(Self { stream })
    }

    /// Send a request and get the response
    fn call(&mut self, method: &str, args: serde_json::Value) -> MadrpcResult<Response> {
        let request = Request::new(method, args);

        // Encode the request as JSON
        let encoded = serde_json::to_vec(&request)
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to encode request: {}", e),
            ))?;

        // Send length prefix (4 bytes, big-endian)
        let len = encoded.len() as u32;
        self.stream
            .write_all(&len.to_be_bytes())
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to send length: {}", e),
            ))?;

        // Send the request data
        self.stream
            .write_all(&encoded)
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to send request: {}", e),
            ))?;
        self.stream
            .flush()
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to flush: {}", e),
            ))?;

        // Read response length
        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to read length: {}", e),
            ))?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;

        // Read response data
        let mut resp_buf = vec![0u8; resp_len];
        self.stream
            .read_exact(&mut resp_buf)
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to read response: {}", e),
            ))?;

        // Decode response
        let response = serde_json::from_slice::<Response>(&resp_buf)
            .map_err(|e| madrpc_common::protocol::error::MadrpcError::Connection(
                format!("Failed to decode response: {}", e),
            ))?;

        Ok(response)
    }
}

/// Start a test server in a background thread
fn start_test_server(script_content: &str) -> (thread::JoinHandle<()>, String) {
    let script_path = create_test_script(script_content);
    let node = Node::new(script_path).expect("Failed to create node");

    // Bind to port 0 to get a random available port
    let server =
        TcpServerThreaded::new("127.0.0.1:0", 4).expect("Failed to create server");
    let addr = server
        .local_addr()
        .expect("Failed to get local address");
    let addr_str = addr.to_string();

    // Start server in background thread
    let handle = thread::spawn(move || {
        // Give the server a short time to start, then stop it
        // We'll use a channel to signal when to stop
        let _ = server.run_with_handler(move |request| node.handle_request(&request));
    });

    // Give the server time to start listening
    thread::sleep(Duration::from_millis(100));

    (handle, addr_str)
}

// ============================================================================
// Built-in Procedure Tests
// ============================================================================

#[test]
fn test_builtin_info() {
    let script = r#"
        madrpc.register('test_func', function(args) {
            return { result: "test" };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    // Give server time to fully start
    thread::sleep(Duration::from_millis(100));

    // Connect and call _info
    let mut client = TestClient::connect(&addr).expect("Failed to connect");
    let response = client
        .call("_info", json!({}))
        .expect("Failed to call _info");

    assert!(response.success, "_info request failed: {:?}", response.error);
    assert!(response.result.is_some());

    let result = response.result.unwrap();
    assert_eq!(result["server_type"], "node");
    assert!(result["uptime_ms"].is_number());
}

#[test]
fn test_builtin_metrics() {
    let script = r#"
        madrpc.register('add', function(args) {
            return { sum: args.a + args.b };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    // Give server time to fully start
    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    // Make a few calls to generate some metrics
    client.call("add", json!({"a": 1, "b": 2})).expect("First call failed");
    client.call("add", json!({"a": 3, "b": 4})).expect("Second call failed");

    // Now call _metrics
    let response = client
        .call("_metrics", json!({}))
        .expect("_metrics call failed");

    assert!(response.success, "_metrics request failed: {:?}", response.error);
    assert!(response.result.is_some());

    let result = response.result.unwrap();
    assert!(result["total_requests"].is_number());
    assert_eq!(result["total_requests"], 2); // 2 add calls (metrics is built-in, not counted)

    // Check method-specific metrics
    if let Some(methods) = result.get("methods") {
        if let Some(add_metrics) = methods.get("add") {
            assert_eq!(add_metrics["call_count"], 2);
        }
    }
}

// ============================================================================
// JavaScript RPC Function Tests
// ============================================================================

#[test]
fn test_js_rpc_basic() {
    let script = r#"
        madrpc.register('echo', function(args) {
            return args;
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call("echo", json!({"msg": "hello", "value": 42}))
        .expect("echo call failed");

    assert!(response.success, "echo request failed: {:?}", response.error);
    assert_eq!(response.result, Some(json!({"msg": "hello", "value": 42})));
}

#[test]
fn test_js_rpc_computation() {
    let script = r#"
        madrpc.register('multiply', function(args) {
            return { product: args.x * args.y };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call("multiply", json!({"x": 7, "y": 6}))
        .expect("multiply call failed");

    assert!(response.success);
    assert_eq!(response.result, Some(json!({"product": 42})));
}

#[test]
fn test_js_rpc_array_manipulation() {
    let script = r#"
        madrpc.register('sum_array', function(args) {
            let arr = args.numbers || [];
            let sum = 0;
            for (let i = 0; i < arr.length; i++) {
                sum += arr[i];
            }
            return { sum: sum };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call("sum_array", json!({"numbers": [1, 2, 3, 4, 5]}))
        .expect("sum_array call failed");

    assert!(response.success);
    assert_eq!(response.result, Some(json!({"sum": 15})));
}

#[test]
fn test_js_rpc_string_manipulation() {
    let script = r#"
        madrpc.register('reverse_string', function(args) {
            let str = args.text || "";
            let reversed = str.split("").reverse().join("");
            return { reversed: reversed, length: str.length };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call("reverse_string", json!({"text": "hello"}))
        .expect("reverse_string call failed");

    assert!(response.success);
    assert_eq!(
        response.result,
        Some(json!({"reversed": "olleh", "length": 5}))
    );
}

#[test]
fn test_js_rpc_error_handling() {
    let script = r#"
        madrpc.register('throws_error', function(args) {
            throw new Error("Intentional error for testing");
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call("throws_error", json!({}))
        .expect("throws_error call failed");

    assert!(!response.success);
    assert!(response.error.is_some());
    let error_msg = response.error.unwrap();
    assert!(error_msg.contains("Intentional error"));
}

#[test]
fn test_js_rpc_nonexistent_method() {
    let script = r#"
        madrpc.register('existing_method', function(args) {
            return { result: "exists" };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call("nonexistent_method", json!({}))
        .expect("nonexistent_method call failed");

    assert!(!response.success);
    assert!(response.error.is_some());
}

#[test]
fn test_js_rpc_complex_object() {
    let script = r#"
        madrpc.register('process_user', function(args) {
            let user = args.user || {};
            return {
                greeting: "Hello, " + user.name + "!",
                is_admin: user.is_admin || false,
                score: (user.score || 0) * 2
            };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    let response = client
        .call(
            "process_user",
            json!({"user": {"name": "Alice", "is_admin": true, "score": 5}}),
        )
        .expect("process_user call failed");

    assert!(response.success);
    assert_eq!(
        response.result,
        Some(json!({
            "greeting": "Hello, Alice!",
            "is_admin": true,
            "score": 10
        }))
    );
}

// ============================================================================
// Multiple Sequential Requests
// ============================================================================

#[test]
fn test_multiple_requests_same_connection() {
    let script = r#"
        madrpc.register('double', function(args) {
            return { doubled: args.value * 2 };
        });

        madrpc.register('triple', function(args) {
            return { tripled: args.value * 3 };
        });
    "#;

    let (_handle, addr) = start_test_server(script);

    thread::sleep(Duration::from_millis(100));

    let mut client = TestClient::connect(&addr).expect("Failed to connect");

    // Make multiple requests on the same connection
    for i in 1..=5 {
        let response = client
            .call("double", json!({"value": i}))
            .expect(&format!("Request {} failed", i));
        assert!(response.success);
        assert_eq!(response.result, Some(json!({"doubled": i * 2})));

        let response = client
            .call("triple", json!({"value": i}))
            .expect(&format!("Request {} failed", i));
        assert!(response.success);
        assert_eq!(response.result, Some(json!({"tripled": i * 3})));
    }
}
