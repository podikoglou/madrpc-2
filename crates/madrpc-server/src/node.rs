use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::{Result, MadrpcError};
use crate::runtime::MadrpcContext;
use madrpc_metrics::{MetricsCollector, NodeMetricsCollector};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// MaDRPC Node - runs Boa and executes JavaScript RPCs
pub struct Node {
    script_path: PathBuf,
    metrics_collector: Arc<NodeMetricsCollector>,
}

impl Node {
    /// Create a new node with a JavaScript script
    pub fn new(script_path: PathBuf) -> Result<Self> {
        Self::with_thread_limit(script_path, num_cpus::get() * 2)
    }

    /// Create a new node with a specific thread limit
    pub fn with_thread_limit(script_path: PathBuf, _max_threads: usize) -> Result<Self> {
        // Validate the script path exists
        if !script_path.exists() {
            return Err(MadrpcError::InvalidRequest(format!(
                "Script path does not exist: {}",
                script_path.display()
            )));
        }

        // Initialize metrics collector
        let metrics_collector = Arc::new(NodeMetricsCollector::new());

        Ok(Self {
            script_path,
            metrics_collector,
        })
    }

    /// Handle an incoming RPC request (synchronous)
    ///
    /// Each worker thread calls this method, which creates a fresh Boa Context
    /// on the calling thread. This ensures thread safety without needing a pool.
    pub fn handle_request(&self, request: &Request) -> Result<Response> {
        tracing::debug!("Handling request for method: {}", request.method);

        // Check for metrics/info requests first
        if self.metrics_collector.is_metrics_request(&request.method) {
            return self.metrics_collector.handle_metrics_request(&request.method, request.id);
        }

        let start_time = Instant::now();
        let method = request.method.clone();

        // Create Boa context on THIS thread (not pooled)
        // This is the key change - context is created fresh each time
        tracing::debug!("Creating new Boa context for request...");
        let ctx = MadrpcContext::new(&self.script_path)?;

        // Call the RPC function
        tracing::debug!("Calling RPC method: {}", method);
        let result = ctx.call_rpc(&method, request.args.clone());
        tracing::debug!("RPC call completed");

        // Context is dropped here

        match result {
            Ok(result) => {
                self.metrics_collector.record_call(&method, start_time, true);
                Ok(Response::success(request.id, result))
            }
            Err(e) => {
                self.metrics_collector.record_call(&method, start_time, false);
                Ok(Response::error(request.id, e.to_string()))
            }
        }
    }

    /// Get the script path
    pub fn script_path(&self) -> &PathBuf {
        &self.script_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::path::PathBuf;
    use std::sync::Mutex;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn create_test_script(content: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!("/tmp/test_madrpc_node_{}.js", id));
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    #[ignore = "Boa 0.20 has a destructor bug that causes these tests to fail during cleanup"]
    fn test_node_creation() {
        let script = create_test_script("// empty");
        let node = Node::new(script);
        assert!(node.is_ok());
    }

    #[test]
    #[ignore = "Boa 0.20 has a destructor bug that causes these tests to fail during cleanup"]
    fn test_node_handles_request() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let node = Node::new(script).unwrap();

        let request = Request::new("echo", json!({"msg": "hello"}));
        let response = node.handle_request(&request).unwrap();

        if !response.success {
            eprintln!("Error: {:?}", response.error);
        }
        assert!(response.success, "Response was not successful: {:?}", response.error);
        assert_eq!(response.result, Some(json!({"msg": "hello"})));
    }

    #[test]
    #[ignore = "Boa 0.20 has a destructor bug that causes these tests to fail during cleanup"]
    fn test_node_returns_error_on_invalid_method() {
        let script = create_test_script("// no functions");
        let node = Node::new(script).unwrap();

        let request = Request::new("nonexistent", json!({}));
        let response = node.handle_request(&request).unwrap();

        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    #[ignore = "Boa 0.20 has a destructor bug that causes these tests to fail during cleanup"]
    fn test_node_returns_js_execution_error() {
        let script = create_test_script(r#"
            madrpc.register('broken', function() {
                throw new Error('intentional error');
            });
        "#);
        let node = Node::new(script).unwrap();

        let request = Request::new("broken", json!({}));
        let response = node.handle_request(&request).unwrap();

        assert!(!response.success);
    }

    #[test]
    #[ignore = "Boa 0.20 has a destructor bug that causes these tests to fail during cleanup"]
    fn test_node_with_computation() {
        let script = create_test_script(r#"
            madrpc.register('compute', function(args) {
                return { result: args.x * args.y };
            });
        "#);
        let node = Node::new(script).unwrap();

        let request = Request::new("compute", json!({"x": 7, "y": 6}));
        let response = node.handle_request(&request).unwrap();

        assert!(response.success);
        assert_eq!(response.result, Some(json!({"result": 42})));
    }

    #[test]
    #[ignore = "Boa 0.20 has a destructor bug that causes these tests to fail during cleanup"]
    fn test_node_concurrent_requests() {
        let script = create_test_script(r#"
            madrpc.register('compute', function(args) {
                return { result: args.x * args.y };
            });
        "#);
        let node = Arc::new(Node::new(script).unwrap());

        // Spawn 3 concurrent requests using std::thread
        let mut handles = Vec::new();
        let results = Arc::new(Mutex::new(Vec::new()));

        for i in 0..3 {
            let node = Arc::clone(&node);
            let results = Arc::clone(&results);
            let handle = std::thread::spawn(move || {
                let request = Request::new("compute", json!({"x": i, "y": 2}));
                let response = node.handle_request(&request).unwrap();
                assert!(response.success);
                results.lock().unwrap().push(response.result);
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(results.lock().unwrap().len(), 3);
    }
}
