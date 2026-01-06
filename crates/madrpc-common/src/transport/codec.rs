use crate::protocol::{Request, Response};
use crate::protocol::error::Result;

/// Codec for encoding/decoding RPC messages
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
