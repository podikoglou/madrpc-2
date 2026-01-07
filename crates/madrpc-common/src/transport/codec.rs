use crate::protocol::{Request, Response};
use crate::protocol::error::Result;

/// Codec for encoding/decoding RPC messages
///
/// Currently only JSON is supported, but the enum allows for future
/// extensibility (e.g., MessagePack, CBOR, etc.).
///
/// # Example
///
/// ```
/// use madrpc_common::transport::Codec;
/// use madrpc_common::protocol::Request;
/// use serde_json::json;
///
/// let codec = Codec::new();
/// let request = Request::new("test", json!({"n": 42}));
///
/// // Encode
/// let encoded = codec.encode_request(&request).unwrap();
///
/// // Decode
/// let decoded = codec.decode_request(&encoded).unwrap();
/// ```
pub enum Codec {
    /// JSON codec (currently the only supported format)
    Json(JsonCodec),
}

impl Codec {
    /// Create a new codec (JSON is the only supported format)
    pub fn new() -> Self {
        Codec::Json(JsonCodec)
    }

    /// Encode a request to bytes
    ///
    /// # Arguments
    ///
    /// * `request` - The request to encode
    ///
    /// # Returns
    ///
    /// JSON-encoded request as a byte vector
    pub fn encode_request(&self, request: &Request) -> Result<Vec<u8>> {
        match self {
            Codec::Json(_) => JsonCodec::encode_request(request),
        }
    }

    /// Decode a request from bytes
    ///
    /// # Arguments
    ///
    /// * `data` - The JSON-encoded request data
    ///
    /// # Returns
    ///
    /// The decoded request
    pub fn decode_request(&self, data: &[u8]) -> Result<Request> {
        match self {
            Codec::Json(_) => JsonCodec::decode_request(data),
        }
    }

    /// Encode a response to bytes
    ///
    /// # Arguments
    ///
    /// * `response` - The response to encode
    ///
    /// # Returns
    ///
    /// JSON-encoded response as a byte vector
    pub fn encode_response(&self, response: &Response) -> Result<Vec<u8>> {
        match self {
            Codec::Json(_) => JsonCodec::encode_response(response),
        }
    }

    /// Decode a response from bytes
    ///
    /// # Arguments
    ///
    /// * `data` - The JSON-encoded response data
    ///
    /// # Returns
    ///
    /// The decoded response
    pub fn decode_response(&self, data: &[u8]) -> Result<Response> {
        match self {
            Codec::Json(_) => JsonCodec::decode_response(data),
        }
    }
}

/// JSON codec for encoding/decoding RPC messages
///
/// Uses JSON serialization for compatibility with `serde_json::Value` types
/// used in `Request` args and `Response` result fields.
///
/// # Example
///
/// ```
/// use madrpc_common::transport::JsonCodec;
/// use madrpc_common::protocol::{Request, Response};
/// use serde_json::json;
///
/// // Encode/decode requests
/// let request = Request::new("test", json!({"n": 42}));
/// let encoded = JsonCodec::encode_request(&request).unwrap();
/// let decoded = JsonCodec::decode_request(&encoded).unwrap();
///
/// // Encode/decode responses
/// let response = Response::success(123, json!({"result": "ok"}));
/// let encoded = JsonCodec::encode_response(&response).unwrap();
/// let decoded = JsonCodec::decode_response(&encoded).unwrap();
/// ```
pub struct JsonCodec;

impl JsonCodec {
    /// Encode a request to bytes
    ///
    /// # Arguments
    ///
    /// * `request` - The request to encode
    ///
    /// # Returns
    ///
    /// JSON-encoded request as a byte vector
    pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(request)?)
    }

    /// Decode a request from bytes
    ///
    /// # Arguments
    ///
    /// * `data` - The JSON-encoded request data
    ///
    /// # Returns
    ///
    /// The decoded request
    pub fn decode_request(data: &[u8]) -> Result<Request> {
        Ok(serde_json::from_slice(data)?)
    }

    /// Encode a response to bytes
    ///
    /// # Arguments
    ///
    /// * `response` - The response to encode
    ///
    /// # Returns
    ///
    /// JSON-encoded response as a byte vector
    pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(response)?)
    }

    /// Decode a response from bytes
    ///
    /// # Arguments
    ///
    /// * `data` - The JSON-encoded response data
    ///
    /// # Returns
    ///
    /// The decoded response
    pub fn decode_response(data: &[u8]) -> Result<Response> {
        Ok(serde_json::from_slice(data)?)
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_codec_round_trip() {
        let request = Request::new("test_method", json!({"arg": 42}));

        let encoded = JsonCodec::encode_request(&request).unwrap();
        let decoded = JsonCodec::decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }

    #[test]
    fn test_json_codec_response_round_trip() {
        let response = Response::success(123, json!({"result": "success"}));

        let encoded = JsonCodec::encode_response(&response).unwrap();
        let decoded = JsonCodec::decode_response(&encoded).unwrap();

        assert_eq!(response, decoded);
    }

    #[test]
    fn test_codec_enum_json() {
        let request = Request::new("test_method", json!({"arg": 42}));
        let codec = Codec::new();

        let encoded = codec.encode_request(&request).unwrap();
        let decoded = codec.decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }

    #[test]
    fn test_request_with_timeout() {
        let request = Request::new("test_method", json!({})).with_timeout(5000);

        let encoded = JsonCodec::encode_request(&request).unwrap();
        let decoded = JsonCodec::decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
        assert_eq!(decoded.timeout_ms, Some(5000));
    }

    #[test]
    fn test_error_response() {
        let response = Response::error(123, "Test error message");

        let encoded = JsonCodec::encode_response(&response).unwrap();
        let decoded = JsonCodec::decode_response(&encoded).unwrap();

        assert_eq!(response, decoded);
        assert!(!decoded.success);
        assert_eq!(decoded.error, Some("Test error message".to_string()));
    }

    #[test]
    fn test_complex_json_values() {
        let request = Request::new(
            "complex_method",
            json!({
                "nested": {
                    "array": [1, 2, 3, "four", null],
                    "boolean": true,
                    "number": 42.5,
                    "string": "test"
                },
                "null_value": null
            })
        );

        let encoded = JsonCodec::encode_request(&request).unwrap();
        let decoded = JsonCodec::decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }
}
