//! Integration tests for the transport layer
//!
//! These tests verify codec functionality for encoding/decoding
//! requests and responses.

#[cfg(test)]
mod tests {
    use crate::protocol::{Request, Response};
    use crate::transport::JsonCodec;
    use serde_json::json;

    #[test]
    fn test_encode_decode_request() {
        let original = Request::new("test_method", json!({"arg": 42, "data": "hello"}));

        let encoded = JsonCodec::encode_request(&original).unwrap();
        assert!(!encoded.is_empty());

        let decoded = JsonCodec::decode_request(&encoded).unwrap();
        assert_eq!(original.method, decoded.method);
        assert_eq!(original.args, decoded.args);
        assert_eq!(original.id, decoded.id);
    }

    #[test]
    fn test_encode_decode_response() {
        let original = Response::success(123, json!({"result": 42, "status": "ok"}));

        let encoded = JsonCodec::encode_response(&original).unwrap();
        assert!(!encoded.is_empty());

        let decoded = JsonCodec::decode_response(&encoded).unwrap();
        assert_eq!(original.id, decoded.id);
        assert_eq!(original.success, decoded.success);
        assert_eq!(original.result, decoded.result);
    }

    #[test]
    fn test_encode_decode_error_response() {
        let original = Response::error(999, "test error message");

        let encoded = JsonCodec::encode_response(&original).unwrap();
        let decoded = JsonCodec::decode_response(&encoded).unwrap();

        assert_eq!(original.id, decoded.id);
        assert!(!decoded.success);
        assert_eq!(decoded.error, Some("test error message".to_string()));
    }

    #[test]
    fn test_invalid_request_data_returns_error() {
        let invalid_data = vec![0xFF, 0xFF, 0xFF, 0xFF];
        let result = JsonCodec::decode_request(&invalid_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_response_data_returns_error() {
        let invalid_data = vec![0x00];
        let result = JsonCodec::decode_response(&invalid_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_request_with_timeout() {
        let original = Request::new("timed", json!({})).with_timeout(5000);
        let encoded = JsonCodec::encode_request(&original).unwrap();
        let decoded = JsonCodec::decode_request(&encoded).unwrap();
        assert_eq!(decoded.timeout_ms, Some(5000));
    }
}
