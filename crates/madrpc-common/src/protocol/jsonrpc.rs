//! JSON-RPC 2.0 Protocol Types
//!
//! This module implements the JSON-RPC 2.0 specification for MaDRPC.
//!
//! # JSON-RPC 2.0 Compliance
//!
//! This implementation follows the JSON-RPC 2.0 specification:
//! - JSON-RPC version: "2.0"
//! - Request format: `{"jsonrpc": "2.0", "method": "...", "params": ..., "id": ...}`
//! - Response format: `{"jsonrpc": "2.0", "result": ..., "error": ..., "id": ...}`
//! - Error format: `{"code": ..., "message": "...", "data": ...}`
//!
//! # Error Codes
//!
//! Standard JSON-RPC 2.0 error codes:
//! - `-32700`: Parse error
//! - `-32600`: Invalid request
//! - `-32601`: Method not found
//! - `-32602`: Invalid params
//! - `-32603`: Internal error
//! - `-32000` to `-32099`: Server error
//!
//! # Example
//!
//! ```
//! use madrpc_common::protocol::jsonrpc::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
//! use serde_json::json;
//!
//! // Create a request
//! let request = JsonRpcRequest {
//!     jsonrpc: "2.0".into(),
//!     method: "compute".into(),
//!     params: json!({"n": 100}),
//!     id: json!(1),
//! };
//!
//! // Create a success response
//! let response = JsonRpcResponse::success(json!(1), json!({"result": 42}));
//!
//! // Create an error response
//! let error = JsonRpcError::method_not_found();
//! let error_response = JsonRpcResponse::error(json!(1), error);
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request
///
/// Per the JSON-RPC 2.0 spec, a request must have:
/// - `jsonrpc`: "2.0"
/// - `method`: String containing the method name to invoke
/// - `params`: Structured value (array or object) holding parameter values
/// - `id`: Request identifier (can be number, string, or null for notifications)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// Name of the method to invoke
    pub method: String,
    /// Parameter values (can be an array or object, or omitted)
    pub params: Value,
    /// Request identifier (number, string, or null)
    pub id: Value,
}

/// JSON-RPC 2.0 response
///
/// Per the JSON-RPC 2.0 spec, a response must have:
/// - `jsonrpc`: "2.0"
/// - `result`: Success result (must not exist if error is present)
/// - `error`: Error object (must not exist if result is present)
/// - `id`: Request identifier (must match the request id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (must be "2.0")
    pub jsonrpc: String,
    /// Result value on success (None if error is present)
    pub result: Option<Value>,
    /// Error object on failure (None if result is present)
    pub error: Option<JsonRpcError>,
    /// Request identifier (must match the request id)
    pub id: Value,
}

/// JSON-RPC 2.0 error
///
/// Per the JSON-RPC 2.0 spec, an error object must have:
/// - `code`: Integer error code
/// - `message`: Short description of the error
/// - `data`: Additional data (optional)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    /// Error code (standard codes are negative integers)
    pub code: i32,
    /// Short description of the error
    pub message: String,
    /// Additional data (optional)
    pub data: Option<Value>,
}

// Standard JSON-RPC 2.0 error codes
/// Invalid JSON was received by the server
pub const PARSE_ERROR: i32 = -32700;
/// The JSON sent is not a valid Request object
pub const INVALID_REQUEST: i32 = -32600;
/// The method does not exist / is not available
pub const METHOD_NOT_FOUND: i32 = -32601;
/// Invalid method parameter(s)
pub const INVALID_PARAMS: i32 = -32602;
/// Internal JSON-RPC error
pub const INTERNAL_ERROR: i32 = -32603;
/// Request entity too large
pub const REQUEST_TOO_LARGE: i32 = -32001;

impl JsonRpcError {
    /// Create a parse error (-32700)
    ///
    /// Used when the server received invalid JSON.
    pub fn parse_error() -> Self {
        Self {
            code: PARSE_ERROR,
            message: "Parse error".into(),
            data: None,
        }
    }

    /// Create an invalid request error (-32600)
    ///
    /// Used when the JSON sent is not a valid Request object.
    pub fn invalid_request() -> Self {
        Self {
            code: INVALID_REQUEST,
            message: "Invalid Request".into(),
            data: None,
        }
    }

    /// Create a method not found error (-32601)
    ///
    /// Used when the method does not exist or is not available.
    pub fn method_not_found() -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: "Method not found".into(),
            data: None,
        }
    }

    /// Create an invalid params error (-32602)
    ///
    /// Used when invalid method parameter(s) were provided.
    pub fn invalid_params(msg: &str) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: msg.into(),
            data: None,
        }
    }

    /// Create an internal error (-32603)
    ///
    /// Used for internal JSON-RPC errors.
    pub fn internal_error(msg: &str) -> Self {
        Self {
            code: INTERNAL_ERROR,
            message: msg.into(),
            data: None,
        }
    }

    /// Create a server error (-32000)
    ///
    /// Used for application-defined errors in the -32000 to -32099 range.
    pub fn server_error(msg: &str) -> Self {
        Self {
            code: -32000,
            message: msg.into(),
            data: None,
        }
    }

    /// Create a request too large error (-32001)
    ///
    /// Used when the request body exceeds the maximum allowed size.
    pub fn request_too_large(limit: usize) -> Self {
        Self {
            code: REQUEST_TOO_LARGE,
            message: format!("Request body too large (max {} bytes)", limit),
            data: None,
        }
    }
}

impl JsonRpcResponse {
    /// Create a success response
    ///
    /// # Arguments
    ///
    /// * `id` - Request identifier (must match the request id)
    /// * `result` - Result value
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response
    ///
    /// # Arguments
    ///
    /// * `id` - Request identifier (must match the request id)
    /// * `error` - Error object
    pub fn error(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "test".into(),
            params: json!({"foo": "bar"}),
            id: json!(1),
        };
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(serialized.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized.contains("\"method\":\"test\""));
        assert!(serialized.contains("\"params\":{"));
        assert!(serialized.contains("\"id\":1"));
    }

    #[test]
    fn test_jsonrpc_response_success() {
        let res = JsonRpcResponse::success(json!(1), json!({"result": 42}));
        assert_eq!(res.result, Some(json!({"result": 42})));
        assert_eq!(res.error, None);
        assert_eq!(res.jsonrpc, "2.0");
        assert_eq!(res.id, json!(1));
    }

    #[test]
    fn test_jsonrpc_response_error() {
        let err = JsonRpcError::method_not_found();
        let res = JsonRpcResponse::error(json!(1), err);
        assert_eq!(res.result, None);
        assert!(res.error.is_some());
        assert_eq!(res.jsonrpc, "2.0");
        assert_eq!(res.id, json!(1));
    }

    #[test]
    fn test_jsonrpc_error_codes() {
        assert_eq!(JsonRpcError::parse_error().code, -32700);
        assert_eq!(JsonRpcError::invalid_request().code, -32600);
        assert_eq!(JsonRpcError::method_not_found().code, -32601);
        assert_eq!(JsonRpcError::invalid_params("test").code, -32602);
        assert_eq!(JsonRpcError::internal_error("test").code, -32603);
        assert_eq!(JsonRpcError::server_error("test").code, -32000);
    }

    #[test]
    fn test_jsonrpc_error_messages() {
        assert_eq!(JsonRpcError::parse_error().message, "Parse error");
        assert_eq!(JsonRpcError::invalid_request().message, "Invalid Request");
        assert_eq!(JsonRpcError::method_not_found().message, "Method not found");
        assert_eq!(JsonRpcError::invalid_params("bad params").message, "bad params");
        assert_eq!(JsonRpcError::internal_error("oops").message, "oops");
        assert_eq!(JsonRpcError::server_error("server error").message, "server error");
    }

    #[test]
    fn test_jsonrpc_request_deserialization() {
        let json = r#"{"jsonrpc":"2.0","method":"test","params":{"foo":"bar"},"id":1}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "test");
        assert_eq!(req.params, json!({"foo": "bar"}));
        assert_eq!(req.id, json!(1));
    }

    #[test]
    fn test_jsonrpc_response_deserialization() {
        let json = r#"{"jsonrpc":"2.0","result":{"value":42},"error":null,"id":1}"#;
        let res: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(res.jsonrpc, "2.0");
        assert_eq!(res.result, Some(json!({"value": 42})));
        assert_eq!(res.error, None);
        assert_eq!(res.id, json!(1));
    }

    #[test]
    fn test_jsonrpc_response_with_error_deserialization() {
        let json = r#"{"jsonrpc":"2.0","result":null,"error":{"code":-32601,"message":"Method not found","data":null},"id":1}"#;
        let res: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(res.jsonrpc, "2.0");
        assert_eq!(res.result, None);
        assert!(res.error.is_some());
        assert_eq!(res.error.unwrap().code, -32601);
        assert_eq!(res.id, json!(1));
    }

    #[test]
    fn test_request_too_large_error() {
        let error = JsonRpcError::request_too_large(1024);
        assert_eq!(error.code, REQUEST_TOO_LARGE);
        assert_eq!(error.code, -32001);
        assert!(error.message.contains("1024"));
        assert!(error.message.contains("too large"));
        assert_eq!(error.data, None);
    }
}
