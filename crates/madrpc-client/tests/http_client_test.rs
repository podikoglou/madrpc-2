//! HTTP Client Integration Tests
//!
//! These tests verify the HTTP client's ability to:
//! - Make JSON-RPC 2.0 calls over HTTP
//! - Handle success and error responses
//! - Retry on transient errors
//! - Not retry on permanent errors
//! - Handle concurrent requests
//!
//! # URL Format Requirements
//!
//! All test URLs must use valid address formats:
//! - Use `127.0.0.1` for IPv4 loopback (not `localhost` to avoid DNS resolution issues)
//! - Use `http://127.0.0.1:PORT` format with explicit port numbers
//! - Avoid IPv6 addresses like `[::1]:PORT` as they may not be valid on all systems
//! - Always include the `http://` or `https://` prefix
//!
//! Examples:
//! - ✅ `http://127.0.0.1:8080`
//! - ✅ `http://127.0.0.1:19999`
//! - ❌ `http://[::1]:8080` (IPv6 may not work on all systems)
//! - ❌ `127.0.0.1:8080` (missing http:// prefix)

use madrpc_client::MadrpcClient;
use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;
use hyper::{Response, Request, StatusCode};
use hyper::service::service_fn;
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use http_body_util::{Full, BodyExt};
use hyper::body::{Bytes, Incoming};

/// Test JSON-RPC server that runs on a separate task
struct TestJsonRpcServer {
    addr: String,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TestJsonRpcServer {
    /// Starts a new test server on a random port
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let handler = Self::default_handler;

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _)) => {
                                let io = TokioIo::new(stream);
                                let handler = handler.clone();

                                tokio::spawn(async move {
                                    let service = service_fn(move |req| {
                                        let handler = handler.clone();
                                        async move { handler(req).await }
                                    });

                                    if let Err(err) = http1::Builder::new()
                                        .serve_connection(io, service)
                                        .await
                                    {
                                        eprintln!("Server error: {}", err);
                                    }
                                });
                            }
                            Err(err) => {
                                eprintln!("Accept error: {}", err);
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    /// Default handler that echoes back params
    async fn default_handler(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
        // Read request body
        let whole_body = req.into_body().collect().await.unwrap().to_bytes();
        let jsonrpc_req: JsonRpcRequest = serde_json::from_slice(&whole_body).unwrap();

        // Echo back the params as result
        let response = JsonRpcResponse::success(jsonrpc_req.id, jsonrpc_req.params);

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(serde_json::to_vec(&response).unwrap())))
            .unwrap())
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for TestJsonRpcServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

// ============================================================================
// Basic Functionality Tests
// ============================================================================

#[tokio::test]
async fn test_client_basic_call() {
    let server = TestJsonRpcServer::new().await;
    let client = MadrpcClient::new(&server.base_url()).await.unwrap();

    let params = json!({"echo": "hello"});
    let result = client.call("echo", params.clone()).await.unwrap();

    assert_eq!(result, params);
}

#[tokio::test]
async fn test_client_call_with_null_params() {
    let server = TestJsonRpcServer::new().await;
    let client = MadrpcClient::new(&server.base_url()).await.unwrap();

    let result = client.call("test", json!(null)).await.unwrap();

    assert_eq!(result, json!(null));
}

#[tokio::test]
async fn test_client_call_with_array_params() {
    let server = TestJsonRpcServer::new().await;
    let client = MadrpcClient::new(&server.base_url()).await.unwrap();

    let params = json!([1, 2, 3, "test"]);
    let result = client.call("test", params.clone()).await.unwrap();

    assert_eq!(result, params);
}

#[tokio::test]
async fn test_client_clone_shares_connection() {
    let server = TestJsonRpcServer::new().await;
    let client = MadrpcClient::new(&server.base_url()).await.unwrap();
    let client2 = client.clone();

    let params1 = json!({"client": 1});
    let params2 = json!({"client": 2});

    let (result1, result2) = tokio::join!(
        client.call("test", params1.clone()),
        client2.call("test", params2.clone())
    );

    assert_eq!(result1.unwrap(), params1);
    assert_eq!(result2.unwrap(), params2);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_client_handles_jsonrpc_error() {
    // Custom server that returns JSON-RPC errors
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        let handler = Arc::new(|req: Request<Incoming>| async move {
            let whole_body = req.into_body().collect().await.unwrap().to_bytes();
            let jsonrpc_req: JsonRpcRequest = serde_json::from_slice(&whole_body).unwrap();

            // Return method not found error
            let error = JsonRpcError::method_not_found();
            let response = JsonRpcResponse::error(jsonrpc_req.id, error);

            Ok::<_, hyper::Error>(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(serde_json::to_vec(&response).unwrap())))
                .unwrap())
        });

        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);
                let handler = handler.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let handler = handler.clone();
                        async move { handler(req).await }
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();

    let result = client.call("nonexistent", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Method not found") ||
            err.to_string().contains("Invalid request"));
}

#[tokio::test]
async fn test_client_no_retry_on_permanent_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    tokio::spawn(async move {
        let handler = Arc::new({
            let call_count = call_count_clone.clone();
            move |req: Request<Incoming>| {
                let call_count = call_count.clone();
                async move {
                    call_count.fetch_add(1, Ordering::SeqCst);
                    let whole_body = req.into_body().collect().await.unwrap().to_bytes();
                    let jsonrpc_req: JsonRpcRequest = serde_json::from_slice(&whole_body).unwrap();

                    // Return invalid params error (permanent)
                    let error = JsonRpcError::invalid_params("bad params");
                    let response = JsonRpcResponse::error(jsonrpc_req.id, error);

                    Ok::<_, hyper::Error>(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(serde_json::to_vec(&response).unwrap())))
                        .unwrap())
                }
            }
        });

        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);
                let handler = handler.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let handler = handler.clone();
                        async move { handler(req).await }
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();

    let result = client.call("test", json!({"bad": "params"})).await;

    assert!(result.is_err());

    // Should only be called once (no retries for permanent errors)
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

// ============================================================================
// Retry Logic Tests
// ============================================================================

#[tokio::test]
async fn test_client_retry_on_transient_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    tokio::spawn(async move {
        let handler = Arc::new({
            let call_count = call_count_clone.clone();
            move |req: Request<Incoming>| {
                let call_count = call_count.clone();
                async move {
                    let count = call_count.fetch_add(1, Ordering::SeqCst);

                    // Fail first two attempts, succeed on third
                    if count < 2 {
                        return Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::new(Bytes::from("server error")))
                            .unwrap());
                    }

                    // Third attempt succeeds
                    let whole_body = req.into_body().collect().await.unwrap().to_bytes();
                    let jsonrpc_req: JsonRpcRequest = serde_json::from_slice(&whole_body).unwrap();
                    let response = JsonRpcResponse::success(jsonrpc_req.id, jsonrpc_req.params);

                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(serde_json::to_vec(&response).unwrap())))
                        .unwrap())
                }
            }
        });

        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);
                let handler = handler.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let handler = handler.clone();
                        async move { handler(req).await }
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();

    let params = json!({"test": "retry"});
    let result = client.call("test", params.clone()).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), params);

    // Should have been called 3 times (initial + 2 retries)
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

// ============================================================================
// Concurrent Requests Tests
// ============================================================================

#[tokio::test]
async fn test_client_concurrent_calls() {
    let server = TestJsonRpcServer::new().await;
    let client = MadrpcClient::new(&server.base_url()).await.unwrap();

    let tasks = (0..10).map(|i| {
        let client = client.clone();
        tokio::spawn(async move {
            let params = json!({"index": i});
            client.call("test", params).await
        })
    }).collect::<Vec<_>>();

    let results = futures::future::join_all(tasks).await;

    for (i, result) in results.into_iter().enumerate() {
        assert!(result.is_ok());
        let rpc_result = result.unwrap().unwrap();
        assert_eq!(rpc_result, json!({"index": i}));
    }
}

// ============================================================================
// Configuration Tests
// ============================================================================

#[tokio::test]
async fn test_client_with_custom_retry_config() {
    let server = TestJsonRpcServer::new().await;

    use madrpc_client::RetryConfig;
    let retry_config = RetryConfig::new(5, 50, 1000, 1.5).unwrap();

    let client = MadrpcClient::with_retry_config(&server.base_url(), retry_config).await.unwrap();

    let params = json!({"test": "config"});
    let result = client.call("test", params.clone()).await.unwrap();

    assert_eq!(result, params);
}

#[tokio::test]
async fn test_client_builder_pattern() {
    let server = TestJsonRpcServer::new().await;

    use madrpc_client::RetryConfig;
    let retry_config = RetryConfig::new(2, 100, 500, 2.0).unwrap();

    let client = MadrpcClient::new(&server.base_url())
        .await
        .unwrap()
        .with_retry(retry_config);

    let params = json!({"test": "builder"});
    let result = client.call("test", params.clone()).await.unwrap();

    assert_eq!(result, params);
}

// ============================================================================
// Error Path Tests
// ============================================================================

#[tokio::test]
async fn test_client_connection_refused() {
    // Try to connect to a server that doesn't exist
    let client = MadrpcClient::new("http://127.0.0.1:19999").await.unwrap();

    let result = client.call("test", json!({"key": "value"})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Should be a retryable error (connection or transport)
    assert!(err.is_retryable());
}

#[tokio::test]
#[ignore = "This test takes too long due to OS-level TCP timeouts"]
async fn test_client_connection_timeout() {
    // This test verifies that the client properly handles connection timeouts
    // We use a non-routable IP address to trigger a timeout
    // Note: This test is ignored by default because it can take a long time
    // depending on OS TCP timeout settings
    let client = MadrpcClient::new("http://192.0.2.1:8080").await.unwrap();

    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Connection errors should be retryable
    assert!(err.is_retryable());
}

#[tokio::test]
async fn test_client_malformed_json_response() {
    // Server that returns malformed JSON
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Full::new(Bytes::from("this is not json")))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Invalid response should not be retryable
    assert!(!err.is_retryable());
    assert!(err.to_string().contains("Invalid response") ||
            err.to_string().contains("parse") ||
            err.to_string().contains("JSON"));
}

#[tokio::test]
async fn test_client_empty_response() {
    // Server that returns empty response
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Full::new(Bytes::from("")))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Invalid response should not be retryable
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn test_client_invalid_jsonrpc_response() {
    // Server that returns invalid JSON-RPC response (missing required fields)
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        // Invalid JSON-RPC: missing jsonrpc version
                        let invalid_response = json!({"result": "ok"});
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Full::new(Bytes::from(serde_json::to_vec(&invalid_response).unwrap())))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Invalid response should not be retryable
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn test_client_http_error_response() {
    // Server that returns HTTP error status
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .body(Full::new(Bytes::from("Bad request")))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // 4xx errors should not be retryable
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn test_client_http_server_error_with_retry() {
    // Server that returns 500 error twice, then succeeds
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    tokio::spawn(async move {
        let handler = Arc::new({
            let call_count = call_count_clone.clone();
            move |req: Request<Incoming>| {
                let call_count = call_count.clone();
                async move {
                    let count = call_count.fetch_add(1, Ordering::SeqCst);

                    // Return 500 for first two attempts
                    if count < 2 {
                        return Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::new(Bytes::from("Internal server error")))
                            .unwrap());
                    }

                    // Third attempt succeeds
                    let whole_body = req.into_body().collect().await.unwrap().to_bytes();
                    let jsonrpc_req: JsonRpcRequest = serde_json::from_slice(&whole_body).unwrap();
                    let response = JsonRpcResponse::success(jsonrpc_req.id, jsonrpc_req.params);

                    Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(serde_json::to_vec(&response).unwrap())))
                        .unwrap())
                }
            }
        });

        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);
                let handler = handler.clone();

                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let handler = handler.clone();
                        async move { handler(req).await }
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let params = json!({"retry": "success"});
    let result = client.call("test", params.clone()).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), params);
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_client_retry_exhausted() {
    // Server that always fails
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::new(Bytes::from("Server error")))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Should have exhausted all retries
    assert!(err.to_string().contains("exhausted") ||
            err.to_string().contains("attempts") ||
            err.to_string().contains("HTTP"));
}

#[tokio::test]
async fn test_client_response_with_wrong_id() {
    // Server that returns response with wrong request ID
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        // Return response with wrong ID
                        let response = JsonRpcResponse::success(serde_json::Value::Number(999.into()), json!({}));
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Full::new(Bytes::from(serde_json::to_vec(&response).unwrap())))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    // The client will accept the response even with wrong ID
    // This test documents current behavior
    let result = client.call("test", json!({})).await;

    // Currently the client doesn't validate response IDs
    // This test documents that behavior
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_truncated_response_body() {
    // Server that returns truncated JSON
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(|_req| async move {
                        // Return truncated JSON
                        Ok::<_, hyper::Error>(Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Full::new(Bytes::from(r#"{"jsonrpc":"2.0","result":{"#)))
                            .unwrap())
                    });

                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = MadrpcClient::new(&base_url).await.unwrap();
    let result = client.call("test", json!({})).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Invalid response should not be retryable
    assert!(!err.is_retryable());
    assert!(err.to_string().contains("Invalid response") ||
            err.to_string().contains("parse") ||
            err.to_string().contains("JSON"));
}

#[tokio::test]
async fn test_client_concurrent_connection_failures() {
    // Test multiple concurrent requests all failing
    let client = MadrpcClient::new("http://127.0.0.1:19998").await.unwrap();

    let tasks = (0..5).map(|i| {
        let client = client.clone();
        tokio::spawn(async move {
            client.call(&format!("test_{}", i), json!({})).await
        })
    }).collect::<Vec<_>>();

    let results = futures::future::join_all(tasks).await;

    // All should fail
    for result in results {
        assert!(result.is_ok());
        let rpc_result = result.unwrap();
        assert!(rpc_result.is_err());
        let err = rpc_result.unwrap_err();
        assert!(err.is_retryable());
    }
}
