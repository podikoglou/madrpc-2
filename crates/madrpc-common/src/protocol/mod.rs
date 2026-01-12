//! MaDRPC Protocol Definitions
//!
//! This module defines the core protocol types for MaDRPC, including requests,
//! responses, and error types used throughout the system.
//!
//! # Protocol Types
//!
//! The protocol uses JSON for serialization with the following core types:
//!
//! - **[`Request`]**: RPC requests with method name, arguments, timeout, and idempotency key
//! - **[`Response`]**: RPC responses with result or error
//! - **[`MadrpcError`]**: Comprehensive error type with retryable/non-retryable classification
//! - **[`NodeRegistrationRequest`]**: Node registration request
//! - **[`NodeRegistrationResponse`]**: Node registration response
//!
//! # Type Aliases
//!
//! - [`RequestId`] - Unique identifier (u64) for each request
//! - [`MethodName`] - String name of the RPC method to call
//! - [`RpcArgs`] - JSON value containing method arguments
//! - [`RpcResult`] - JSON value containing method result
//!
//! # Error Handling
//!
//! Errors are classified as retryable or non-retryable:
//! - **Retryable**: Transport errors, timeouts, node unavailable (transient issues)
//! - **Non-retryable**: Invalid requests, JavaScript errors, all nodes failed (permanent issues)
//!
//! # Example
//!
//! ```
//! use madrpc_common::protocol::{Request, Response};
//! use serde_json::json;
//!
//! // Create a request with timeout and idempotency
//! let request = Request::new("compute", json!({"n": 100}))
//!     .with_timeout(5000)
//!     .with_idempotency_key("unique-123");
//!
//! // Create a success response
//! let response = Response::success(request.id, json!({"result": 42}));
//! ```

use serde::{Deserialize, Serialize};

/// Node registration request
///
/// Sent by a node to register itself with an orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistrationRequest {
    /// The URL where the node is accessible
    pub node_url: String,
}

/// Node registration response
///
/// Sent by the orchestrator in response to a node registration request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistrationResponse {
    /// The URL that was registered (may differ from the request URL)
    pub registered_url: String,
    /// Whether this is a new registration or a duplicate
    pub is_new_registration: bool,
}

pub mod builtin;
pub mod error;
pub mod jsonrpc;
pub mod requests;
pub mod responses;

#[cfg(test)]
mod tests;

pub use builtin::*;
pub use error::{MadrpcError, Result};
pub use jsonrpc::{
    JsonRpcRequest, JsonRpcResponse, JsonRpcError,
    PARSE_ERROR, INVALID_REQUEST, METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL_ERROR, REQUEST_TOO_LARGE,
    NODE_REGISTRATION_ERROR
};
pub use requests::{Request, RequestId, MethodName, RpcArgs};
pub use responses::{Response, RpcResult};
