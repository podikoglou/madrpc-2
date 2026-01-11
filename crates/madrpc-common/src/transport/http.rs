//! HTTP Transport Utilities
//!
//! This module provides HTTP-specific utilities for the JSON-RPC protocol.
//!
//! # Architecture
//!
//! The HTTP transport layer provides:
//! - Parsing JSON-RPC requests from HTTP bodies
//! - Creating HTTP responses from JSON-RPC responses
//! - Type aliases for Hyper request/response types
//!
//! # Components
//!
//! - **[`HttpTransport`]**: Utility functions for HTTP/JSON-RPC conversion
//! - **[`HyperRequest`]**: Type alias for Hyper incoming requests
//! - **[`HyperResponse`]**: Type alias for Hyper responses
//!
//! # Example
//!
//! ```no_run
//! use madrpc_common::transport::http::HttpTransport;
//! use hyper::{Request, Response};
//! use http_body_util::Full;
//! use hyper::body::Bytes;
//! use serde_json::json;
//!
//! // Build a JSON-RPC request
//! let jsonrpc_request = HttpTransport::build_request(
//!     "compute",
//!     json!({"n": 100}),
//!     json!(1)
//! ).unwrap();
//!
//! // Convert a JSON-RPC response to HTTP
//! let jsonrpc_response = madrpc_common::protocol::JsonRpcResponse::success(
//!     json!(1),
//!     json!({"result": 42})
//! );
//! let http_response = HttpTransport::to_http_response(jsonrpc_response);
//! ```

use hyper::{Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};

use crate::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use crate::protocol::error::MadrpcError;

/// Maximum payload size in bytes (10 MB)
///
/// This limit prevents memory exhaustion attacks by restricting the size
/// of JSON-RPC request payloads. Requests exceeding this size will be
/// rejected with a `MadrpcError::PayloadTooLarge` error.
///
/// The 10 MB limit is chosen to allow large practical payloads while
/// preventing abuse. Most JSON-RPC requests are much smaller (typically
/// under 1 MB).
pub const MAX_PAYLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Type alias for Hyper incoming requests
pub type HyperRequest = Request<Incoming>;

/// Type alias for Hyper responses with full body
pub type HyperResponse = Response<Full<Bytes>>;

/// HTTP transport utility functions
///
/// Provides conversion between HTTP and JSON-RPC protocol messages.
pub struct HttpTransport;

impl HttpTransport {
    /// Parse a JSON-RPC request from an HTTP body
    ///
    /// # Arguments
    ///
    /// * `body` - Raw HTTP body bytes
    ///
    /// # Returns
    ///
    /// A parsed `JsonRpcRequest` or a `MadrpcError` if parsing fails
    ///
    /// # Errors
    ///
    /// Returns `MadrpcError::PayloadTooLarge` if the body exceeds `MAX_PAYLOAD_SIZE`
    /// Returns `MadrpcError::JsonSerialization` if JSON parsing fails
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::transport::http::HttpTransport;
    /// use hyper::body::Bytes;
    ///
    /// let body = Bytes::from(r#"{"jsonrpc":"2.0","method":"test","params":{},"id":1}"#);
    /// let request = HttpTransport::parse_jsonrpc(body).unwrap();
    /// assert_eq!(request.method, "test");
    /// ```
    pub fn parse_jsonrpc(body: Bytes) -> Result<JsonRpcRequest, MadrpcError> {
        // Validate payload size before parsing
        let body_len = body.len();
        if body_len > MAX_PAYLOAD_SIZE {
            return Err(MadrpcError::PayloadTooLarge(body_len, MAX_PAYLOAD_SIZE));
        }

        serde_json::from_slice(&body).map_err(|e| MadrpcError::JsonSerialization(e))
    }

    /// Create an HTTP response from a JSON-RPC response
    ///
    /// # Arguments
    ///
    /// * `jsonrpc` - JSON-RPC response object
    ///
    /// # Returns
    ///
    /// A Hyper HTTP response with appropriate headers and status code
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::transport::http::HttpTransport;
    /// use madrpc_common::protocol::JsonRpcResponse;
    /// use serde_json::json;
    ///
    /// let jsonrpc_response = JsonRpcResponse::success(json!(1), json!({"result": 42}));
    /// let http_response = HttpTransport::to_http_response(jsonrpc_response);
    /// ```
    pub fn to_http_response(jsonrpc: JsonRpcResponse) -> HyperResponse {
        let body = serde_json::to_vec(&jsonrpc).unwrap_or_default();

        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .unwrap()
    }

    /// Create an HTTP error response from a JSON-RPC error
    ///
    /// # Arguments
    ///
    /// * `id` - Request identifier
    /// * `error` - JSON-RPC error object
    ///
    /// # Returns
    ///
    /// A Hyper HTTP response with appropriate error status code
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::transport::http::HttpTransport;
    /// use madrpc_common::protocol::JsonRpcError;
    /// use serde_json::json;
    ///
    /// let error = JsonRpcError::method_not_found();
    /// let http_response = HttpTransport::to_http_error(json!(1), error);
    /// ```
    pub fn to_http_error(id: serde_json::Value, error: JsonRpcError) -> HyperResponse {
        let jsonrpc_response = JsonRpcResponse::error(id, error);
        Self::to_http_response(jsonrpc_response)
    }

    /// Build a JSON-RPC request
    ///
    /// # Arguments
    ///
    /// * `method` - Method name to invoke
    /// * `params` - Method parameters (can be an object or array)
    /// * `id` - Request identifier
    ///
    /// # Returns
    ///
    /// A properly formatted `JsonRpcRequest`
    ///
    /// # Errors
    ///
    /// Returns `MadrpcError::PayloadTooLarge` if the serialized request
    /// exceeds `MAX_PAYLOAD_SIZE`
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::transport::http::HttpTransport;
    /// use serde_json::json;
    ///
    /// let request = HttpTransport::build_request(
    ///     "compute",
    ///     json!({"n": 100}),
    ///     json!(1)
    /// ).unwrap();
    /// assert_eq!(request.method, "compute");
    /// ```
    pub fn build_request(
        method: &str,
        params: serde_json::Value,
        id: serde_json::Value,
    ) -> Result<JsonRpcRequest, MadrpcError> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id,
        };

        // Validate serialized size
        let serialized = serde_json::to_vec(&request)?;
        if serialized.len() > MAX_PAYLOAD_SIZE {
            return Err(MadrpcError::PayloadTooLarge(serialized.len(), MAX_PAYLOAD_SIZE));
        }

        Ok(request)
    }

    /// Create an HTTP response with a custom status code
    ///
    /// # Arguments
    ///
    /// * `jsonrpc` - JSON-RPC response object
    /// * `status` - HTTP status code
    ///
    /// # Returns
    ///
    /// A Hyper HTTP response with the specified status code
    pub fn to_http_response_with_status(jsonrpc: JsonRpcResponse, status: StatusCode) -> HyperResponse {
        let body = serde_json::to_vec(&jsonrpc).unwrap_or_default();

        Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcError;
    use serde_json::json;

    #[test]
    fn test_parse_jsonrpc_valid_request() {
        let body = Bytes::from(r#"{"jsonrpc":"2.0","method":"test","params":{"foo":"bar"},"id":1}"#);
        let request = HttpTransport::parse_jsonrpc(body).unwrap();
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.method, "test");
        assert_eq!(request.params, json!({"foo": "bar"}));
        assert_eq!(request.id, json!(1));
    }

    #[test]
    fn test_parse_jsonrpc_invalid_json() {
        let body = Bytes::from(r#"{"jsonrpc":"2.0","method":"test","params":}"#);
        let result = HttpTransport::parse_jsonrpc(body);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_http_response_success() {
        let jsonrpc_response = JsonRpcResponse::success(json!(1), json!({"result": 42}));
        let http_response = HttpTransport::to_http_response(jsonrpc_response);

        assert_eq!(http_response.status(), StatusCode::OK);
        assert_eq!(
            http_response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_to_http_response_error() {
        let error = JsonRpcError::method_not_found();
        let jsonrpc_response = JsonRpcResponse::error(json!(1), error);
        let http_response = HttpTransport::to_http_response(jsonrpc_response);

        assert_eq!(http_response.status(), StatusCode::OK);
        assert_eq!(
            http_response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_to_http_error() {
        let error = JsonRpcError::invalid_params("Invalid parameter");
        let http_response = HttpTransport::to_http_error(json!(1), error);

        assert_eq!(http_response.status(), StatusCode::OK);
        assert_eq!(
            http_response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_build_request() {
        let request = HttpTransport::build_request("compute", json!({"n": 100}), json!(1)).unwrap();
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.method, "compute");
        assert_eq!(request.params, json!({"n": 100}));
        assert_eq!(request.id, json!(1));
    }

    #[test]
    fn test_to_http_response_with_status() {
        let jsonrpc_response = JsonRpcResponse::success(json!(1), json!({"result": 42}));
        let http_response = HttpTransport::to_http_response_with_status(
            jsonrpc_response,
            StatusCode::ACCEPTED,
        );

        assert_eq!(http_response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            http_response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_http_response_body_serialization() {
        let jsonrpc_response = JsonRpcResponse::success(json!(1), json!({"result": 42}));
        let _http_response = HttpTransport::to_http_response(jsonrpc_response.clone());

        // Verify the response can be serialized back
        let body_str = serde_json::to_string(&jsonrpc_response).unwrap();

        assert!(body_str.contains(r#""jsonrpc":"2.0""#));
        assert!(body_str.contains(r#""result":"#));
        assert!(body_str.contains(r#""id":1"#));
    }

    #[test]
    fn test_http_error_response_body_serialization() {
        let error = JsonRpcError::method_not_found();
        let jsonrpc_response = JsonRpcResponse::error(json!(1), error);
        let _http_response = HttpTransport::to_http_response(jsonrpc_response.clone());

        // Verify the response can be serialized back
        let body_str = serde_json::to_string(&jsonrpc_response).unwrap();

        assert!(body_str.contains(r#""jsonrpc":"2.0""#));
        assert!(body_str.contains(r#""error":"#));
        assert!(body_str.contains(r#""code":-32601"#));
        assert!(body_str.contains(r#""message":"Method not found""#));
    }

    #[test]
    fn test_parse_jsonrpc_normal_size() {
        let body = Bytes::from(r#"{"jsonrpc":"2.0","method":"test","params":{"foo":"bar"},"id":1}"#);
        let result = HttpTransport::parse_jsonrpc(body);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().method, "test");
    }

    #[test]
    fn test_parse_jsonrpc_exactly_at_limit() {
        // Create a payload that is exactly at the limit (minus the JSON-RPC wrapper overhead)
        // We'll create a payload that's close to but under the limit
        let data = "x".repeat(MAX_PAYLOAD_SIZE - 100);
        let json_str = format!(r#"{{"jsonrpc":"2.0","method":"test","params":{{"data":"{}"}},"id":1}}"#, data);
        let body = Bytes::from(json_str);

        let result = HttpTransport::parse_jsonrpc(body);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_jsonrpc_exceeds_limit() {
        // Create a payload that exceeds the limit
        let data = "x".repeat(MAX_PAYLOAD_SIZE + 1);
        let json_str = format!(r#"{{"jsonrpc":"2.0","method":"test","params":{{"data":"{}"}},"id":1}}"#, data);
        let body = Bytes::from(json_str);

        let result = HttpTransport::parse_jsonrpc(body);
        assert!(result.is_err());

        match result {
            Err(MadrpcError::PayloadTooLarge(size, limit)) => {
                assert!(size > limit);
                assert_eq!(limit, MAX_PAYLOAD_SIZE);
            }
            _ => panic!("Expected PayloadTooLarge error"),
        }
    }

    #[test]
    fn test_build_request_normal_size() {
        let result = HttpTransport::build_request("test", json!({"foo": "bar"}), json!(1));
        assert!(result.is_ok());
        let request = result.unwrap();
        assert_eq!(request.method, "test");
    }

    #[test]
    fn test_build_request_exceeds_limit() {
        // Create params that exceed the size limit when serialized
        let large_data = "x".repeat(MAX_PAYLOAD_SIZE);
        let result = HttpTransport::build_request("test", json!({"data": large_data}), json!(1));

        assert!(result.is_err());

        match result {
            Err(MadrpcError::PayloadTooLarge(size, limit)) => {
                assert!(size > limit);
                assert_eq!(limit, MAX_PAYLOAD_SIZE);
            }
            _ => panic!("Expected PayloadTooLarge error"),
        }
    }

    #[test]
    fn test_payload_too_large_error_message() {
        let error = MadrpcError::PayloadTooLarge(15_000_000, 10_485_760);
        let error_string = error.to_string();

        assert!(error_string.contains("15000000"));
        assert!(error_string.contains("10485760"));
        assert!(error_string.contains("exceeds maximum allowed size"));
    }

    #[test]
    fn test_payload_too_large_not_retryable() {
        let error = MadrpcError::PayloadTooLarge(15_000_000, 10_485_760);
        assert!(!error.is_retryable());
    }
}
