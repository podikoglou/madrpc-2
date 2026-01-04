use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::{Result, MadrpcError};
use crate::runtime::MadrpcContext;
use madrpc_metrics::{MetricsCollector, NodeMetricsCollector};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// MaDRPC Node - runs QuickJS and executes JavaScript RPCs
pub struct Node {
    runtime: Arc<MadrpcContext>,
    script_path: PathBuf,
    metrics_collector: Arc<NodeMetricsCollector>,
}

impl Node {
    /// Create a new node with a JavaScript script
    pub async fn new(script_path: PathBuf) -> Result<Self> {
        Self::with_orchestrator(script_path, None).await
    }

    /// Create a new node with optional orchestrator for distributed calls
    pub async fn with_orchestrator(
        script_path: PathBuf,
        _orchestrator_addr: Option<String>,
    ) -> Result<Self> {
        // Create the QuickJS context in a blocking task to avoid async issues
        let script_path_clone = script_path.clone();
        let runtime = tokio::task::spawn_blocking(move || {
            MadrpcContext::with_client(&script_path_clone, None)
        }).await
        .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to create runtime: {}", e)))??;

        let runtime = Arc::new(runtime);

        // Initialize metrics
        let metrics_collector = Arc::new(NodeMetricsCollector::new());

        // TODO: Set up client after runtime creation if orchestrator_addr is provided

        Ok(Self {
            runtime,
            script_path,
            metrics_collector,
        })
    }

    /// Handle an incoming RPC request
    pub async fn handle_request(&self, request: &Request) -> Result<Response> {
        // Check for metrics/info requests
        if self.metrics_collector.is_metrics_request(&request.method) {
            return self.metrics_collector.handle_metrics_request(&request.method, request.id);
        }

        let start_time = Instant::now();
        let method = request.method.clone();

        // Call the registered function in a blocking task
        let runtime = self.runtime.clone();
        let args = request.args.clone();
        let id = request.id;
        let method_clone = method.clone();

        let result = tokio::task::spawn_blocking(move || {
            runtime.call_rpc(&method_clone, args)
        }).await
        .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to execute RPC: {}", e)))?;

        match result {
            Ok(result) => {
                self.metrics_collector.record_call(&method, start_time, true);
                Ok(Response::success(id, result))
            }
            Err(e) => {
                self.metrics_collector.record_call(&method, start_time, false);
                Ok(Response::error(id, e.to_string()))
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

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn create_test_script(content: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!("/tmp/test_madrpc_node_{}.js", id));
        fs::write(&path, content).unwrap();
        path
    }

    #[tokio::test]
    async fn test_node_creation() {
        let script = create_test_script("// empty");
        let node = Node::new(script).await;
        assert!(node.is_ok());
    }

    #[tokio::test]
    async fn test_node_handles_request() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let node = Node::new(script).await.unwrap();

        let request = Request::new("echo", json!({"msg": "hello"}));
        let response = node.handle_request(&request).await.unwrap();

        assert!(response.success);
        assert_eq!(response.result, Some(json!({"msg": "hello"})));
    }

    #[tokio::test]
    async fn test_node_returns_error_on_invalid_method() {
        let script = create_test_script("// no functions");
        let node = Node::new(script).await.unwrap();

        let request = Request::new("nonexistent", json!({}));
        let response = node.handle_request(&request).await.unwrap();

        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[tokio::test]
    async fn test_node_returns_js_execution_error() {
        let script = create_test_script(r#"
            madrpc.register('broken', function() {
                throw new Error('intentional error');
            });
        "#);
        let node = Node::new(script).await.unwrap();

        let request = Request::new("broken", json!({}));
        let response = node.handle_request(&request).await.unwrap();

        assert!(!response.success);
    }

    #[tokio::test]
    async fn test_node_with_computation() {
        let script = create_test_script(r#"
            madrpc.register('compute', function(args) {
                return { result: args.x * args.y };
            });
        "#);
        let node = Node::new(script).await.unwrap();

        let request = Request::new("compute", json!({"x": 7, "y": 6}));
        let response = node.handle_request(&request).await.unwrap();

        assert!(response.success);
        assert_eq!(response.result, Some(json!({"result": 42})));
    }
}
