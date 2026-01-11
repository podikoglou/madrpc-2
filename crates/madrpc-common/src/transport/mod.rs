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
//! ```
//! use madrpc_common::transport::HttpTransport;
//! use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse};
//! use serde_json::json;
//!
//! // Build a JSON-RPC request
//! let request = HttpTransport::build_request(
//!     "compute",
//!     json!({"n": 100}),
//!     json!(1)
//! ).unwrap();
//!
//! // Create an HTTP response from a JSON-RPC response
//! let jsonrpc_response = JsonRpcResponse::success(
//!     json!(1),
//!     json!({"result": 42})
//! );
//! let http_response = HttpTransport::to_http_response(jsonrpc_response);
//! ```

pub mod http;

pub use http::{HttpTransport, HyperRequest, HyperResponse, MAX_PAYLOAD_SIZE};
