//! MaDRPC Response Types
//!
//! This module defines the RPC response structure.

use serde::{Deserialize, Serialize};
use super::RequestId;

/// RPC method result (JSON value)
///
/// The result is returned as a JSON value and can contain any JSON-serializable data.
pub type RpcResult = serde_json::Value;

/// An RPC response returned from a node to the client.
///
/// # Response Flow
///
/// 1. Node receives and processes a `Request`
/// 2. Node creates a `Response` (success or error)
/// 3. Response is serialized to JSON and sent via TCP
/// 4. Client deserializes and handles the response
///
/// # Fields
///
/// - `id`: The request ID this response corresponds to (for matching requests/responses)
/// - `result`: The result value (present on success)
/// - `error`: Error message (present on failure)
/// - `success`: Whether the request succeeded
///
/// # Example
///
/// ```
/// use madrpc_common::protocol::responses::Response;
/// use serde_json::json;
///
/// // Create a success response
/// let success = Response::success(123, json!({"pi": 3.14159}));
///
/// // Create an error response
/// let error = Response::error(123, "Division by zero");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    /// Request identifier this response corresponds to
    pub id: RequestId,
    /// Result value (present on success)
    pub result: Option<RpcResult>,
    /// Error message (present on failure)
    pub error: Option<String>,
    /// Whether the request succeeded
    pub success: bool,
}

impl Response {
    /// Creates a successful response.
    ///
    /// # Arguments
    ///
    /// * `id` - The request identifier (must match the request's ID)
    /// * `result` - The result value (any JSON-serializable data)
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::protocol::responses::Response;
    /// use serde_json::json;
    ///
    /// let response = Response::success(123, json!({"pi": 3.14159}));
    /// assert!(response.success);
    /// assert_eq!(response.result, Some(json!({"pi": 3.14159})));
    /// ```
    pub fn success(id: RequestId, result: RpcResult) -> Self {
        Response {
            id,
            result: Some(result),
            error: None,
            success: true,
        }
    }

    /// Creates an error response.
    ///
    /// # Arguments
    ///
    /// * `id` - The request identifier (must match the request's ID)
    /// * `error` - The error message (describing what went wrong)
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::protocol::responses::Response;
    ///
    /// let response = Response::error(123, "Division by zero");
    /// assert!(!response.success);
    /// assert_eq!(response.error, Some("Division by zero".to_string()));
    /// ```
    pub fn error(id: RequestId, error: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(error.into()),
            success: false,
        }
    }
}
