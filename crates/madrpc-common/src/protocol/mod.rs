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

pub mod error;
pub mod requests;
pub mod responses;

#[cfg(test)]
mod tests;

pub use error::{MadrpcError, Result};
pub use requests::{Request, RequestId, MethodName, RpcArgs};
pub use responses::{Response, RpcResult};
