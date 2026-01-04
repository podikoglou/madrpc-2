use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::{Result, MadrpcError};
use crate::runtime::{ContextPool, PoolConfig};
use madrpc_metrics::{MetricsCollector, NodeMetricsCollector};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// MaDRPC Node - runs QuickJS and executes JavaScript RPCs
pub struct Node {
    pool: Arc<ContextPool>,
    script_path: PathBuf,
    metrics_collector: Arc<NodeMetricsCollector>,
}

impl Node {
    /// Create a new node with a JavaScript script
    /// Uses pool_size = 1 by default due to QuickJS threading limitations
    pub async fn new(script_path: PathBuf) -> Result<Self> {
        Self::with_pool_size(script_path, 1).await
    }

    /// Create a new node with a specific pool size
    pub async fn with_pool_size(script_path: PathBuf, pool_size: usize) -> Result<Self> {
        let config = PoolConfig { pool_size };

        // Create the context pool in a blocking task to avoid async issues
        let script_path_clone = script_path.clone();
        let pool = tokio::task::spawn_blocking(move || {
            ContextPool::new(script_path_clone, config)
        }).await
        .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to create context pool: {}", e)))??;

        let pool = Arc::new(pool);

        // Initialize metrics
        let metrics_collector = Arc::new(NodeMetricsCollector::new());

        Ok(Self {
            pool,
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

        // Acquire a context from the pool
        // If pool is exhausted, this will async wait until one becomes available
        let pooled_ctx = self.pool.acquire().await?;

        // Call the registered function in a blocking task
        let method_clone = method.clone();
        let args = request.args.clone();

        let result = tokio::task::spawn_blocking(move || {
            pooled_ctx.call_rpc(&method_clone, args)
        }).await
        .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to execute RPC: {}", e)))?;

        // pooled_ctx is dropped here, automatically releasing it back to the pool

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

        if !response.success {
            eprintln!("Error: {:?}", response.error);
        }
        assert!(response.success, "Response was not successful: {:?}", response.error);
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

    #[tokio::test]
    async fn test_node_concurrent_requests() {
        let script = create_test_script(r#"
            madrpc.register('compute', function(args) {
                return { result: args.x * args.y };
            });
        "#);
        let node = Arc::new(Node::new(script).await.unwrap());

        // Spawn 3 concurrent requests
        let mut tasks = Vec::new();
        for i in 0..3 {
            let node = Arc::clone(&node);
            let task = tokio::spawn(async move {
                let request = Request::new("compute", json!({"x": i, "y": 2}));
                node.handle_request(&request).await
            });
            tasks.push(task);
        }

        // Wait for all results
        let mut results = Vec::new();
        for task in tasks {
            let response = task.await.unwrap().unwrap();
            assert!(response.success);
            results.push(response.result);
        }

        assert_eq!(results.len(), 3);
    }
}
