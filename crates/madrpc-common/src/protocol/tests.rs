#[cfg(test)]
mod tests {
    use super::super::*;
    use serde_json::json;

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
}
