//! Integration tests for the protocol module
//!
//! These tests verify the correctness of request/response serialization,
//! ID generation, and error handling.

#[cfg(test)]
mod tests {
    use super::super::*;
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn test_request_creation() {
        let req = Request::new("test_method", json!({"arg": 42}));
        assert_eq!(req.method, "test_method");
        assert_eq!(req.args, json!({"arg": 42}));
        assert!(req.timeout_ms.is_none());
    }

    #[test]
    fn test_request_with_timeout() {
        let req = Request::new("test", json!({})).with_timeout(5000);
        assert_eq!(req.timeout_ms, Some(5000));
    }

    #[test]
    fn test_request_id_uniqueness() {
        let ids: HashSet<_> = (0..1000)
            .map(|_| Request::new("test", json!({})).id)
            .collect();
        assert_eq!(ids.len(), 1000, "All request IDs should be unique");
    }

    #[test]
    fn test_response_success() {
        let resp = Response::success(123, json!({"result": "ok"}));
        assert!(resp.success);
        assert_eq!(resp.id, 123);
        assert_eq!(resp.result, Some(json!({"result": "ok"})));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_response_error() {
        let resp = Response::error(456, "something failed");
        assert!(!resp.success);
        assert_eq!(resp.id, 456);
        assert_eq!(resp.error, Some("something failed".to_string()));
        assert!(resp.result.is_none());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let req = Request::new("test_method", json!({"x": 1}));
        let serialized = serde_json::to_value(&req).unwrap();
        let deserialized: Request = serde_json::from_value(serialized).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        let resp = Response::success(1, json!({"value": 42}));
        let serialized = serde_json::to_value(&resp).unwrap();
        let deserialized: Response = serde_json::from_value(serialized).unwrap();
        assert_eq!(resp, deserialized);
    }

    // ========================================================================
    // Request ID Stress Tests
    // ========================================================================

    #[test]
    fn test_request_id_uniqueness_under_stress() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let ids = Arc::new(Mutex::new(HashSet::new()));
        let mut handles = vec![];

        // Spawn 10 threads, each creating 1000 requests
        for _ in 0..10 {
            let ids_clone = Arc::clone(&ids);
            let handle = thread::spawn(move || {
                for _ in 0..1000 {
                    let id = Request::new("test", json!({})).id;
                    let mut ids = ids_clone.lock().unwrap();
                    assert!(!ids.contains(&id), "Duplicate ID detected: {}", id);
                    ids.insert(id);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Should have exactly 10,000 unique IDs
        assert_eq!(ids.lock().unwrap().len(), 10_000);
    }

    #[test]
    fn test_request_id_monotonicity() {
        let mut prev_id = Request::new("test", json!({})).id;

        for _ in 0..1000 {
            let id = Request::new("test", json!({})).id;
            // Most IDs should be >= previous (due to counter increment)
            assert!(id >= prev_id || id < 1000, "ID went backward significantly: {} -> {}", prev_id, id);
            prev_id = id;
        }
    }

    #[test]
    fn test_request_id_structure() {
        let id = Request::new("test", json!({})).id;

        // Lower 32 bits should be counter (small, increasing)
        let lower = id & 0xFFFFFFFF;
        assert!(lower < 1_000_000, "Lower 32 bits should be counter: {}", lower);

        // ID should be non-zero
        assert!(id > 0, "ID should be non-zero");
    }
}
