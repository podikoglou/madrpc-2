//! MaDRPC Transport Layer
//!
//! This module provides HTTP transport for sending/receiving JSON-RPC messages.
//!
//! # Architecture
//!
//! The transport layer handles all network communication using:
//! - **Transport**: HTTP/1.1 with JSON-RPC protocol
//! - **Serialization**: JSON for request/response encoding
//! - **Protocol**: Standard JSON-RPC 2.0 specification
//!
//! # Components
//!
//! - **[`HttpTransport`]**: HTTP transport for JSON-RPC requests
//! - **[`HyperRequest`]**: Type alias for hyper Request
//! - **[`HyperResponse`]**: Type alias for hyper Response
//!
//! # Example
//!
//! ```no_run
//! use madrpc_common::transport::HttpTransport;
//! use madrpc_common::protocol::JsonRpcRequest;
//! use serde_json::json;
//!
//! // Create HTTP transport
//! let transport = HttpTransport::new();
//!
//! // Create JSON-RPC request
//! let request = JsonRpcRequest {
//!     jsonrpc: "2.0".to_string(),
//!     method: "compute".to_string(),
//!     params: json!({"n": 100}),
//!     id: 1.into(),
//! };
//!
//! // Send request
//! let response = transport.send_request("127.0.0.1:8080", &request).await.unwrap();
//! ```

pub mod http;

pub use http::{HttpTransport, HyperRequest, HyperResponse};
