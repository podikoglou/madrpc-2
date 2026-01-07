//! MaDRPC Client
//!
//! This crate provides the RPC client for making requests to MaDRPC orchestrators.
//!
//! # Overview
//!
//! The `MadrpcClient` provides a high-level interface for making RPC calls to MaDRPC
//! orchestrators. It handles connection pooling, automatic retries with exponential backoff,
//! and error classification (retryable vs non-retryable errors).
//!
//! # Key Features
//!
//! - **Connection Pooling**: Reuses TCP connections across requests for improved performance
//! - **Automatic Retries**: Transient failures (network issues, timeouts) are automatically
//!   retried with exponential backoff and jitter
//! - **Error Classification**: Distinguishes between retryable and non-retryable errors
//! - **Configurable**: Custom pool and retry configurations for different use cases
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
//!     let client = MadrpcClient::new("127.0.0.1:8080").await?;
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
//! - Connection pool exhaustion
//!
//! Permanent errors (e.g., invalid requests, JavaScript execution errors) are not retried
//! and fail immediately.
//!
//! # Connection Pooling
//!
//! The client maintains a pool of TCP connections to each orchestrator. By default,
//! up to 10 connections are pooled per address, with a 30-second timeout for acquiring
//! connections from the pool.

pub mod pool;
pub mod client;

pub use pool::{ConnectionPool, PoolConfig, PooledConnection};
pub use client::{MadrpcClient, RetryConfig};
