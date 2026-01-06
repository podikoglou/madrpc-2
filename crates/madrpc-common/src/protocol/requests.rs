//! MaDRPC Request Types

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Unique identifier for an RPC request
pub type RequestId = u64;

/// Name of the RPC method to call
pub type MethodName = String;

/// RPC method arguments (JSON value)
pub type RpcArgs = serde_json::Value;

static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An RPC request to be sent from client to node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    /// Unique request identifier
    pub id: RequestId,
    /// Method name to call
    pub method: MethodName,
    /// Method arguments
    pub args: RpcArgs,
    /// Optional timeout in milliseconds
    pub timeout_ms: Option<u64>,
    /// Optional idempotency key for safe retries
    pub idempotency_key: Option<String>,
}

impl Request {
    /// Creates a new RPC request.
    ///
    /// # Arguments
    /// * `method` - The method name to call
    /// * `args` - The method arguments as JSON value
    pub fn new(method: impl Into<String>, args: RpcArgs) -> Self {
        Request {
            id: generate_request_id(),
            method: method.into(),
            args,
            timeout_ms: None,
            idempotency_key: None,
        }
    }

    /// Sets the timeout for this request.
    ///
    /// # Arguments
    /// * `timeout_ms` - Timeout in milliseconds
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Sets an idempotency key for safe retries.
    ///
    /// # Arguments
    /// * `key` - Unique key for idempotency
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

fn generate_request_id() -> RequestId {
    // Try to use system time as the base
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // Always increment the counter to ensure uniqueness
    let counter = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

    // Combine timestamp and counter to ensure uniqueness
    // Use the lower 32 bits for counter and upper 32 bits for timestamp
    (timestamp & 0xFFFFFFFF00000000) | (counter & 0xFFFFFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_request_creation() {
        let req = Request::new("test_method", json!({"arg": 42}));
        assert_eq!(req.method, "test_method");
        assert_eq!(req.args, json!({"arg": 42}));
        assert!(req.timeout_ms.is_none());
        assert!(req.idempotency_key.is_none());
    }

    #[test]
    fn test_request_with_timeout() {
        let req = Request::new("test", json!({})).with_timeout(5000);
        assert_eq!(req.timeout_ms, Some(5000));
    }

    #[test]
    fn test_request_with_idempotency_key() {
        let req = Request::new("test", json!({}))
            .with_idempotency_key("unique-key-123");
        assert_eq!(req.idempotency_key, Some("unique-key-123".to_string()));
    }

    #[test]
    fn test_request_with_timeout_and_idempotency() {
        let req = Request::new("test", json!({}))
            .with_timeout(3000)
            .with_idempotency_key("key-456");
        assert_eq!(req.timeout_ms, Some(3000));
        assert_eq!(req.idempotency_key, Some("key-456".to_string()));
    }

    #[test]
    fn test_request_id_uniqueness() {
        let req1 = Request::new("test1", json!({}));
        let req2 = Request::new("test2", json!({}));
        assert_ne!(req1.id, req2.id);
    }

    #[test]
    fn test_request_serialization() {
        let req = Request::new("test", json!({"x": 1}))
            .with_timeout(1000)
            .with_idempotency_key("key-123");

        let serialized = serde_json::to_string(&req).unwrap();
        let deserialized: Request = serde_json::from_str(&serialized).unwrap();

        assert_eq!(req.method, deserialized.method);
        assert_eq!(req.args, deserialized.args);
        assert_eq!(req.timeout_ms, deserialized.timeout_ms);
        assert_eq!(req.idempotency_key, deserialized.idempotency_key);
    }
}
