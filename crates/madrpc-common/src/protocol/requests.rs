//! MaDRPC Request Types
//!
//! This module defines the RPC request structure and unique ID generation.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Unique identifier for an RPC request
///
/// Each request is assigned a unique 64-bit ID that combines a timestamp
/// with a counter to ensure uniqueness across the system.
pub type RequestId = u64;

/// Name of the RPC method to call
///
/// This is the string name of the JavaScript function registered on the node.
pub type MethodName = String;

/// RPC method arguments (JSON value)
///
/// Arguments are passed as a JSON value and can contain any JSON-serializable data.
pub type RpcArgs = serde_json::Value;

/// Global counter for ensuring unique request IDs
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An RPC request to be sent from client to node.
///
/// # Request Flow
///
/// 1. Client creates a `Request` with method name and arguments
/// 2. Request is serialized to JSON and sent via TCP
/// 3. Node deserializes and executes the method
/// 4. Node returns a `Response` with the same request ID
///
/// # Fields
///
/// - `id`: Unique identifier (combines timestamp + counter for uniqueness)
/// - `method`: Name of the JavaScript function to call
/// - `args`: Arguments to pass to the function (JSON value)
/// - `timeout_ms`: Optional timeout for request execution
/// - `idempotency_key`: Optional key for safe retry of requests
///
/// # Example
///
/// ```
/// use madrpc_common::protocol::requests::Request;
/// use serde_json::json;
///
/// // Create a basic request
/// let request = Request::new("compute", json!({"n": 100}));
///
/// // Create a request with timeout
/// let request = Request::new("compute", json!({"n": 100}))
///     .with_timeout(5000);
///
/// // Create a request with idempotency for safe retries
/// let request = Request::new("transfer", json!({"amount": 100}))
///     .with_timeout(5000)
///     .with_idempotency_key("txn-123");
/// ```
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
    /// Automatically generates a unique request ID by combining the current
    /// timestamp with an atomic counter.
    ///
    /// # Arguments
    ///
    /// * `method` - The method name to call (e.g., "compute_pi")
    /// * `args` - The method arguments as a JSON value
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::protocol::requests::Request;
    /// use serde_json::json;
    ///
    /// let request = Request::new("compute", json!({"n": 100}));
    /// assert_eq!(request.method, "compute");
    /// assert_eq!(request.args, json!({"n": 100}));
    /// ```
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
    /// If the request takes longer than the specified timeout, it will be
    /// cancelled and a timeout error will be returned.
    ///
    /// # Arguments
    ///
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::protocol::requests::Request;
    /// use serde_json::json;
    ///
    /// let request = Request::new("compute", json!({}))
    ///     .with_timeout(5000);
    /// assert_eq!(request.timeout_ms, Some(5000));
    /// ```
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Sets an idempotency key for safe retries.
    ///
    /// Idempotency keys ensure that retrying a request won't cause duplicate
    /// operations. The server should track processed keys and return cached
    /// results for duplicate keys.
    ///
    /// # Arguments
    ///
    /// * `key` - Unique key for idempotency (e.g., UUID, transaction ID)
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::protocol::requests::Request;
    /// use serde_json::json;
    ///
    /// let request = Request::new("transfer", json!({"amount": 100}))
    ///     .with_idempotency_key("txn-abc-123");
    /// assert_eq!(request.idempotency_key, Some("txn-abc-123".to_string()));
    /// ```
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

/// Generates a unique request ID.
///
/// The ID combines:
/// - Upper 32 bits: Timestamp (seconds since UNIX epoch, valid until 2106)
/// - Lower 32 bits: Atomic counter
///
/// This ensures uniqueness across restarts and within the same runtime.
/// The 32-bit second counter provides sufficient range until year 2106.
fn generate_request_id() -> RequestId {
    // Try to use system time as the base (seconds since UNIX epoch)
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Always increment the counter to ensure uniqueness
    // Relaxed ordering is sufficient since we only need uniqueness, not synchronization
    let counter = REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Combine timestamp and counter to ensure uniqueness
    // Upper 32 bits: timestamp, Lower 32 bits: counter
    (timestamp << 32) | (counter & 0xFFFFFFFF)
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

    #[test]
    fn test_request_id_no_collisions_under_concurrency() {
        use std::collections::HashSet;
        use std::thread;

        // Generate 10,000 request IDs across 10 threads
        let num_threads = 10;
        let ids_per_thread = 1000;
        let mut handles = vec![];

        for _ in 0..num_threads {
            let handle = thread::spawn(move || {
                let mut ids = HashSet::new();
                for _ in 0..ids_per_thread {
                    let id = generate_request_id();
                    // Check for collision within this thread
                    assert!(
                        ids.insert(id),
                        "Duplicate request ID detected: {}",
                        id
                    );
                }
                ids
            });
            handles.push(handle);
        }

        // Collect all IDs from all threads
        let mut all_ids = HashSet::new();
        for handle in handles {
            let thread_ids = handle.join().unwrap();
            for id in thread_ids {
                // Check for collision across threads
                assert!(
                    all_ids.insert(id),
                    "Duplicate request ID detected across threads: {}",
                    id
                );
            }
        }

        // Verify we generated the expected number of unique IDs
        assert_eq!(
            all_ids.len(),
            num_threads * ids_per_thread,
            "Total unique IDs should match expected count"
        );
    }

    #[test]
    fn test_request_id_structure() {
        // Generate an ID and verify its structure
        let id = generate_request_id();

        // Extract timestamp (upper 32 bits) and counter (lower 32 bits)
        let timestamp = id >> 32;
        let counter = id & 0xFFFFFFFF;

        // Verify timestamp is reasonable (between 2020 and 2106)
        // Current timestamp (2026) is around 1737487700 seconds
        assert!(
            timestamp > 1577836800,
            "Timestamp should be after 2020-01-01"
        );
        assert!(
            timestamp < 4294967296,
            "Timestamp should fit in 32 bits (valid until 2106)"
        );

        // Verify counter is in valid range
        assert!(counter < 4294967296, "Counter should fit in 32 bits");
    }
}
