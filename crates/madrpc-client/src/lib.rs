//! MaDRPC Client
//!
//! This crate provides the RPC client for making requests to MaDRPC orchestrators.
//!
//! # Overview
//!
//! The `MadrpcClient` provides a high-level interface for making JSON-RPC 2.0 calls to MaDRPC
//! orchestrators via HTTP. It handles automatic retries with exponential backoff and error
//! classification (retryable vs non-retryable errors).
//!
//! # Key Features
//!
//! - **HTTP/JSON-RPC 2.0**: Uses HTTP with JSON-RPC 2.0 protocol
//! - **Connection Keep-Alive**: Hyper's HTTP/1.1 keep-alive handles connection reuse
//! - **Automatic Retries**: Transient failures (network issues, timeouts) are automatically
//!   retried with exponential backoff and jitter
//! - **Error Classification**: Distinguishes between retryable and non-retryable errors
//! - **Configurable**: Custom retry configurations for different use cases
//!
//! # Usage
//!
//! ```rust,no_run
//! use madrpc_client::MadrpcClient;
//! use serde_json::json;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a client with default configuration
//!     let client = MadrpcClient::new("http://127.0.0.1:8080").await?;
//!
//!     // Make an RPC call
//!     let result = client.call("my_method", json!({"arg": 42})).await?;
//!
//!     println!("Result: {}", result);
//!     Ok(())
//! }
//! ```
//!
//! # Retry Behavior
//!
//! By default, the client will retry up to 3 times for transient errors such as:
//! - Network transport failures
//! - Request timeouts
//! - Node unavailability
//!
//! Permanent errors (e.g., invalid requests, JavaScript execution errors) are not retried
//! and fail immediately.
//!
//! # Connection Management
//!
//! The client uses hyper's HTTP client with HTTP/1.1 keep-alive for efficient connection
//! reuse. Connections are automatically pooled and reused by hyper's connection pool.

pub mod client;

pub use client::{MadrpcClient, RetryConfig, RetryConfigError};
