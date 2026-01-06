//! MaDRPC Response Types

use serde::{Deserialize, Serialize};
use super::RequestId;

/// RPC method result (JSON value)
pub type RpcResult = serde_json::Value;

/// An RPC response returned from a node to the client.
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
    /// * `id` - The request identifier
    /// * `result` - The result value
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
    /// * `id` - The request identifier
    /// * `error` - The error message
    pub fn error(id: RequestId, error: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(error.into()),
            success: false,
        }
    }
}
