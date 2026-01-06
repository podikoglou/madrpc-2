use crate::protocol::{Request, Response};
use crate::protocol::error::Result;

/// Serialization format for RPC messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerializationFormat {
    /// JSON format - human-readable, good for debugging (format byte: 0x01)
    Json = 0x01,
    /// MessagePack format - compact binary, better performance (format byte: 0x02)
    MessagePack = 0x02,
}

impl SerializationFormat {
    /// Parse from format byte
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(SerializationFormat::Json),
            0x02 => Some(SerializationFormat::MessagePack),
            _ => None,
        }
    }

    /// Get format byte
    pub fn as_byte(&self) -> u8 {
        *self as u8
    }
}

/// Codec for encoding/decoding RPC messages
///
/// Supports both JSON (for compatibility/debugging) and MessagePack (for performance)
pub enum Codec {
    Json(JsonCodec),
    MessagePack(MessagePackCodec),
}

impl Codec {
    /// Create a new codec for the specified format
    pub fn new(format: SerializationFormat) -> Self {
        match format {
            SerializationFormat::Json => Codec::Json(JsonCodec),
            SerializationFormat::MessagePack => Codec::MessagePack(MessagePackCodec),
        }
    }

    /// Encode a request to bytes
    pub fn encode_request(&self, request: &Request) -> Result<Vec<u8>> {
        match self {
            Codec::Json(_) => JsonCodec::encode_request(request),
            Codec::MessagePack(_) => MessagePackCodec::encode_request(request),
        }
    }

    /// Decode a request from bytes
    pub fn decode_request(&self, data: &[u8]) -> Result<Request> {
        match self {
            Codec::Json(_) => JsonCodec::decode_request(data),
            Codec::MessagePack(_) => MessagePackCodec::decode_request(data),
        }
    }

    /// Encode a response to bytes
    pub fn encode_response(&self, response: &Response) -> Result<Vec<u8>> {
        match self {
            Codec::Json(_) => JsonCodec::encode_response(response),
            Codec::MessagePack(_) => MessagePackCodec::encode_response(response),
        }
    }

    /// Decode a response from bytes
    pub fn decode_response(&self, data: &[u8]) -> Result<Response> {
        match self {
            Codec::Json(_) => JsonCodec::decode_response(data),
            Codec::MessagePack(_) => MessagePackCodec::decode_response(data),
        }
    }

    /// Get the serialization format
    pub fn format(&self) -> SerializationFormat {
        match self {
            Codec::Json(_) => SerializationFormat::Json,
            Codec::MessagePack(_) => SerializationFormat::MessagePack,
        }
    }
}

/// JSON codec for encoding/decoding RPC messages
///
/// Uses JSON serialization for compatibility with serde_json::Value types
/// used in Request args and Response result fields.
pub struct JsonCodec;

impl JsonCodec {
    /// Encode a request to bytes
    pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(request)?)
    }

    /// Decode a request from bytes
    pub fn decode_request(data: &[u8]) -> Result<Request> {
        Ok(serde_json::from_slice(data)?)
    }

    /// Encode a response to bytes
    pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(response)?)
    }

    /// Decode a response from bytes
    pub fn decode_response(data: &[u8]) -> Result<Response> {
        Ok(serde_json::from_slice(data)?)
    }
}

/// MessagePack codec for encoding/decoding RPC messages
///
/// Uses MessagePack serialization for better performance and smaller message size.
/// MessagePack is a binary serialization format that is self-describing and supports
/// serde types including serde_json::Value for dynamic data.
pub struct MessagePackCodec;

impl MessagePackCodec {
    /// Encode a request to bytes
    pub fn encode_request(request: &Request) -> Result<Vec<u8>> {
        rmp_serde::to_vec(request)
            .map_err(|e| crate::protocol::error::MadrpcError::MessagePackSerialization(e.to_string()))
    }

    /// Decode a request from bytes
    pub fn decode_request(data: &[u8]) -> Result<Request> {
        rmp_serde::from_slice(data)
            .map_err(|e| crate::protocol::error::MadrpcError::MessagePackSerialization(e.to_string()))
    }

    /// Encode a response to bytes
    pub fn encode_response(response: &Response) -> Result<Vec<u8>> {
        rmp_serde::to_vec(response)
            .map_err(|e| crate::protocol::error::MadrpcError::MessagePackSerialization(e.to_string()))
    }

    /// Decode a response from bytes
    pub fn decode_response(data: &[u8]) -> Result<Response> {
        rmp_serde::from_slice(data)
            .map_err(|e| crate::protocol::error::MadrpcError::MessagePackSerialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_serialization_format_from_byte() {
        assert_eq!(
            SerializationFormat::from_byte(0x01),
            Some(SerializationFormat::Json)
        );
        assert_eq!(
            SerializationFormat::from_byte(0x02),
            Some(SerializationFormat::MessagePack)
        );
        assert_eq!(SerializationFormat::from_byte(0xFF), None);
    }

    #[test]
    fn test_serialization_format_as_byte() {
        assert_eq!(SerializationFormat::Json.as_byte(), 0x01);
        assert_eq!(SerializationFormat::MessagePack.as_byte(), 0x02);
    }

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
    fn test_messagepack_codec_round_trip() {
        let request = Request::new("test_method", json!({"arg": 42}));

        let encoded = MessagePackCodec::encode_request(&request).unwrap();
        let decoded = MessagePackCodec::decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }

    #[test]
    fn test_messagepack_codec_response_round_trip() {
        let response = Response::success(123, json!({"result": "success"}));

        let encoded = MessagePackCodec::encode_response(&response).unwrap();
        let decoded = MessagePackCodec::decode_response(&encoded).unwrap();

        assert_eq!(response, decoded);
    }

    #[test]
    fn test_messagepack_smaller_than_json() {
        let request = Request::new("test_method", json!({"arg": 42, "name": "test", "data": [1, 2, 3]}));

        let json_encoded = JsonCodec::encode_request(&request).unwrap();
        let messagepack_encoded = MessagePackCodec::encode_request(&request).unwrap();

        // MessagePack should be more compact than JSON
        assert!(
            messagepack_encoded.len() < json_encoded.len(),
            "MessagePack ({} bytes) should be smaller than JSON ({} bytes)",
            messagepack_encoded.len(),
            json_encoded.len()
        );
    }

    #[test]
    fn test_codec_enum_json() {
        let request = Request::new("test_method", json!({"arg": 42}));
        let codec = Codec::new(SerializationFormat::Json);

        assert_eq!(codec.format(), SerializationFormat::Json);

        let encoded = codec.encode_request(&request).unwrap();
        let decoded = codec.decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }

    #[test]
    fn test_codec_enum_messagepack() {
        let request = Request::new("test_method", json!({"arg": 42}));
        let codec = Codec::new(SerializationFormat::MessagePack);

        assert_eq!(codec.format(), SerializationFormat::MessagePack);

        let encoded = codec.encode_request(&request).unwrap();
        let decoded = codec.decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }

    #[test]
    fn test_request_with_timeout() {
        let request = Request::new("test_method", json!({})).with_timeout(5000);

        let encoded = MessagePackCodec::encode_request(&request).unwrap();
        let decoded = MessagePackCodec::decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
        assert_eq!(decoded.timeout_ms, Some(5000));
    }

    #[test]
    fn test_error_response() {
        let response = Response::error(123, "Test error message");

        let encoded = MessagePackCodec::encode_response(&response).unwrap();
        let decoded = MessagePackCodec::decode_response(&encoded).unwrap();

        assert_eq!(response, decoded);
        assert!(!decoded.success);
        assert_eq!(decoded.error, Some("Test error message".to_string()));
    }

    #[test]
    fn test_complex_json_values() {
        // Test that MessagePack handles complex serde_json::Value structures
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

        let encoded = MessagePackCodec::encode_request(&request).unwrap();
        let decoded = MessagePackCodec::decode_request(&encoded).unwrap();

        assert_eq!(request, decoded);
    }

    #[test]
    fn test_messagepack_performance_characteristics() {
        // Verify that MessagePack provides size benefits over JSON for typical data
        let simple_request = Request::new("method", json!({"x": 1, "y": 2}));
        let complex_request = Request::new(
            "method",
            json!({
                "data": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                "metadata": {"name": "test", "version": 1}
            })
        );

        let simple_json = JsonCodec::encode_request(&simple_request).unwrap().len();
        let simple_messagepack = MessagePackCodec::encode_request(&simple_request).unwrap().len();

        let complex_json = JsonCodec::encode_request(&complex_request).unwrap().len();
        let complex_messagepack = MessagePackCodec::encode_request(&complex_request).unwrap().len();

        // MessagePack should be smaller for both cases
        assert!(
            simple_messagepack < simple_json,
            "MessagePack should be smaller for simple requests: {} vs {}",
            simple_messagepack, simple_json
        );
        assert!(
            complex_messagepack < complex_json,
            "MessagePack should be smaller for complex requests: {} vs {}",
            complex_messagepack, complex_json
        );
    }
}
