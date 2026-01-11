use madrpc_common::protocol::{Request, Response, MetricsResponse, InfoResponse, NodeInfo};
use madrpc_common::protocol::error::{Result, MadrpcError};
use crate::runtime::MadrpcContext;
use crate::resource_limits::ResourceLimits;
use madrpc_metrics::{MetricsCollector, NodeMetricsCollector};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// MaDRPC Node - runs Boa and executes JavaScript RPCs.
///
/// Each request creates a fresh Boa Context to enable true parallelism.
/// This is necessary because Boa Context has thread-local state and is
/// not thread-safe.
///
/// # Script Caching Strategy
///
/// The node caches the script source to avoid reading the file on every request.
/// This provides a significant performance improvement by avoiding file I/O.
///
/// Note: We cannot cache the parsed AST or compiled bytecode because Boa's
/// string interner is tied to a specific Context. Each request must parse the
/// script in its own Context, which has its own interner.
///
/// # Resource Limits
///
/// The node enforces resource limits on JavaScript execution to prevent
/// runaway code from consuming excessive resources. By default, execution
/// is limited to 30 seconds per request.
pub struct Node {
    /// Path to the JavaScript script file
    script_path: PathBuf,
    /// Cached script source to avoid reading the file on every request
    script_source: Arc<String>,
    /// Metrics collector for this node
    metrics_collector: Arc<NodeMetricsCollector>,
    /// Optional orchestrator client for distributed RPC calls
    orchestrator_client: Option<Arc<madrpc_client::MadrpcClient>>,
    /// Resource limits for JavaScript execution
    resource_limits: ResourceLimits,
}


impl Node {
    /// Creates a new node with a JavaScript script.
    ///
    /// The script source is read and cached to avoid file I/O on every request.
    /// Each request will parse the script in its own Boa Context to enable
    /// true parallelism.
    ///
    /// # Arguments
    /// * `script_path` - Path to the JavaScript script file
    pub fn new(script_path: PathBuf) -> Result<Self> {
        Self::with_resource_limits(script_path, ResourceLimits::default())
    }

    /// Creates a new node with custom resource limits.
    ///
    /// This constructor allows specifying custom resource limits for JavaScript
    /// execution, such as maximum execution time.
    ///
    /// # Arguments
    /// * `script_path` - Path to the JavaScript script file
    /// * `resource_limits` - Resource limits configuration
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The script file doesn't exist
    /// - The script cannot be read
    /// - The resource limits configuration is invalid
    pub fn with_resource_limits(script_path: PathBuf, resource_limits: ResourceLimits) -> Result<Self> {
        // Validate resource limits
        resource_limits.validate()
            .map_err(|e| MadrpcError::InvalidRequest(format!("Invalid resource limits: {}", e)))?;
        if !script_path.exists() {
            return Err(MadrpcError::InvalidRequest(format!(
                "Script path does not exist: {}",
                script_path.display()
            )));
        }

        // Read and cache the script source to avoid file I/O on every request
        let script_source = std::fs::read_to_string(&script_path)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to load script: {}", e)))?;

        tracing::info!("Script source loaded and cached");

        let metrics_collector = Arc::new(NodeMetricsCollector::new());

        Ok(Self {
            script_path,
            script_source: Arc::new(script_source),
            metrics_collector,
            orchestrator_client: None,
            resource_limits,
        })
    }

    /// Creates a new node with orchestrator support.
    ///
    /// This constructor creates a node that can make distributed RPC calls to other nodes
    /// through the orchestrator. The client is stored and passed to each context created
    /// during request handling.
    ///
    /// # Arguments
    /// * `script_path` - Path to the JavaScript script file
    /// * `orchestrator_addr` - Address of the orchestrator (e.g., "127.0.0.1:8080")
    pub async fn with_orchestrator(script_path: PathBuf, orchestrator_addr: String) -> Result<Self> {
        Self::with_orchestrator_and_resource_limits(
            script_path,
            orchestrator_addr,
            ResourceLimits::default()
        ).await
    }

    /// Creates a new node with orchestrator support and custom resource limits.
    ///
    /// This constructor combines distributed RPC capability with custom resource limits
    /// for maximum control over node behavior.
    ///
    /// # Arguments
    /// * `script_path` - Path to the JavaScript script file
    /// * `orchestrator_addr` - Address of the orchestrator (e.g., "127.0.0.1:8080")
    /// * `resource_limits` - Resource limits configuration
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The script file doesn't exist
    /// - The script cannot be read
    /// - The orchestrator client cannot be created
    /// - The resource limits configuration is invalid
    pub async fn with_orchestrator_and_resource_limits(
        script_path: PathBuf,
        orchestrator_addr: String,
        resource_limits: ResourceLimits,
    ) -> Result<Self> {
        // Validate resource limits
        resource_limits.validate()
            .map_err(|e| MadrpcError::InvalidRequest(format!("Invalid resource limits: {}", e)))?;
        if !script_path.exists() {
            return Err(MadrpcError::InvalidRequest(format!(
                "Script path does not exist: {}",
                script_path.display()
            )));
        }

        // Read and cache the script source to avoid file I/O on every request
        let script_source = std::fs::read_to_string(&script_path)
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to load script: {}", e)))?;

        tracing::info!("Script source loaded and cached");

        let metrics_collector = Arc::new(NodeMetricsCollector::new());

        // Create the orchestrator client (async)
        let orchestrator_client = madrpc_client::MadrpcClient::new(orchestrator_addr).await
            .map_err(|e| MadrpcError::InvalidRequest(format!("Failed to create orchestrator client: {}", e)))?;

        tracing::info!("Orchestrator client created");

        Ok(Self {
            script_path,
            script_source: Arc::new(script_source),
            metrics_collector,
            orchestrator_client: Some(Arc::new(orchestrator_client)),
            resource_limits,
        })
    }

    /// Sets custom resource limits on the node (builder pattern).
    ///
    /// This method allows modifying resource limits after node creation
    /// using the builder pattern.
    ///
    /// # Arguments
    ///
    /// * `resource_limits` - New resource limits configuration
    ///
    /// # Returns
    ///
    /// `Self` for builder pattern chaining.
    ///
    /// # Panics
    ///
    /// Panics if the resource limits configuration is invalid.
    /// Use `ResourceLimits::validate()` to check before calling.
    pub fn and_resource_limits(mut self, resource_limits: ResourceLimits) -> Self {
        resource_limits.validate()
            .expect("Invalid resource limits");
        self.resource_limits = resource_limits;
        self
    }

    /// Gets a reference to the current resource limits.
    pub fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    /// Handles an incoming RPC request.
    ///
    /// Creates a fresh Boa Context for each request to enable true parallelism.
    /// Multiple requests can execute concurrently on the same node.
    ///
    /// # Arguments
    /// * `request` - The RPC request to handle
    pub fn handle_request(&self, request: &Request) -> Result<Response> {
        tracing::debug!("Handling request for method: {}", request.method);

        // Check for metrics/info requests first
        if self.metrics_collector.is_metrics_request(&request.method) {
            return self.metrics_collector.handle_metrics_request(&request.method, request.id);
        }

        let start_time = Instant::now();
        let method = request.method.clone();

        // Create a fresh Boa context for this request from the cached script source
        // This enables true parallelism as each request has its own context
        tracing::debug!("Creating fresh Boa context for request");

        // If we have an orchestrator client, pass it to the context for distributed RPC calls
        let mut ctx = if let Some(client) = &self.orchestrator_client {
            // Clone the inner MadrpcClient (MadrpcClient implements Clone)
            let client_clone = (**client).clone();
            MadrpcContext::with_client_from_source(&self.script_source, Some(client_clone))?
        } else {
            MadrpcContext::from_source(&self.script_source)?
        };

        // Call the RPC function
        tracing::debug!("Calling RPC method: {}", method);
        let result = ctx.call_rpc(&method, request.args.clone());
        tracing::debug!("RPC call completed");

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

    /// Gets the script path.
    pub fn script_path(&self) -> &PathBuf {
        &self.script_path
    }

    /// Calls an RPC method asynchronously.
    ///
    /// This method creates a fresh Boa context and executes the requested method.
    /// It's designed for use by the HTTP server which needs async execution.
    ///
    /// # Arguments
    ///
    /// * `method` - The name of the method to call
    /// * `params` - The parameters to pass to the method (as JSON Value)
    ///
    /// # Returns
    ///
    /// A `Result` containing the method's return value as a JSON Value
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The method is not registered
    /// - Method execution fails
    /// - Parameters are invalid
    pub async fn call_rpc(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        // Apply execution timeout from resource limits
        let timeout_duration = self.resource_limits.execution_timeout;

        // Use spawn_blocking to move JavaScript execution to a blocking thread
        // This is necessary because MadrpcContext is !Send (Boa's Context has thread affinity)
        let script_source = self.script_source.clone();
        let method = method.to_string();
        let method_for_metrics = method.clone();
        let orchestrator_client = self.orchestrator_client.clone();
        let metrics_collector = self.metrics_collector.clone();

        let handle = tokio::runtime::Handle::try_current()
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("No tokio runtime: {}", e)))?;

        // Wrap the entire execution in a timeout
        let execution_task = handle.spawn_blocking(move || {
            let start_time = std::time::Instant::now();

            // Create a fresh Boa context for this request on the blocking thread
            let ctx_result = if let Some(client) = orchestrator_client {
                let client_clone = (*client).clone();
                MadrpcContext::with_client_from_source(&script_source, Some(client_clone))
            } else {
                MadrpcContext::from_source(&script_source)
            };

            let mut ctx = match ctx_result {
                Ok(ctx) => ctx,
                Err(e) => return (Err(e), start_time),
            };

            // Block on the async call_rpc_async using a new runtime
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .enable_io()
                .build()
                .unwrap();

            let method_ref = method.as_str();
            let result = runtime.block_on(ctx.call_rpc_async(method_ref, params));

            (result, start_time)
        });

        // Apply timeout to the execution
        let (result, start_time) = tokio::time::timeout(timeout_duration, execution_task)
            .await
            .map_err(|_| MadrpcError::JavaScriptExecution(format!(
                "JavaScript execution exceeded time limit of {}ms",
                timeout_duration.as_millis()
            )))?
            .map_err(|e| MadrpcError::JavaScriptExecution(format!("Task join error: {}", e)))?;

        // Record metrics
        match &result {
            Ok(_) => metrics_collector.record_call(&method_for_metrics, start_time, true),
            Err(_) => metrics_collector.record_call(&method_for_metrics, start_time, false),
        }

        result
    }

    /// Gets the current metrics snapshot.
    ///
    /// # Returns
    ///
    /// A `Result` containing the metrics snapshot
    ///
    /// # Errors
    ///
    /// Returns an error if metrics collection fails
    pub async fn get_metrics(&self) -> Result<MetricsResponse> {
        Ok(self.metrics_collector.snapshot())
    }

    /// Gets server information.
    ///
    /// # Returns
    ///
    /// A `Result` containing the server info
    ///
    /// # Errors
    ///
    /// Returns an error if info collection fails
    pub async fn get_info(&self) -> Result<InfoResponse> {
        let uptime_ms = self.metrics_collector.snapshot().uptime_ms;
        Ok(InfoResponse::Node(NodeInfo::new(uptime_ms)))
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

    #[test]
    fn test_node_creation() {
        let script = create_test_script("// empty");
        let node = Node::new(script);
        assert!(node.is_ok());
    }

    #[test]
    fn test_node_handles_request() {
        let script = create_test_script(r#"
            madrpc.register('echo', function(args) {
                return args;
            });
        "#);
        let node = Node::new(script).unwrap();

        let request = Request::new("echo", json!({"msg": "hello"}));
        let response = node.handle_request(&request).unwrap();

        assert!(response.success, "Response was not successful: {:?}", response.error);
        assert_eq!(response.result, Some(json!({"msg": "hello"})));
    }

    #[test]
    fn test_node_returns_error_on_invalid_method() {
        let script = create_test_script("// no functions");
        let node = Node::new(script).unwrap();

        let request = Request::new("nonexistent", json!({}));
        let response = node.handle_request(&request).unwrap();

        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
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

    #[tokio::test]
    async fn test_node_creation_with_orchestrator() {
        let script = create_test_script("// empty");
        // Note: This test will fail if there's no orchestrator running at the address
        // In a real integration test, you'd start a test orchestrator first
        let result = Node::with_orchestrator(script, "127.0.0.1:9999".to_string()).await;
        // We expect this to fail with a connection error since no orchestrator is running
        // But it validates that the constructor logic works (script loading, etc.)
        match result {
            Ok(_) => {
                // If it succeeds, an orchestrator was running - that's fine too
            }
            Err(e) => {
                // Expected error: "Failed to create orchestrator client: ..."
                // This confirms the constructor reached the client creation stage
                let error_msg = e.to_string();
                assert!(error_msg.contains("Failed to create orchestrator client"),
                    "Expected client creation error, got: {}", error_msg);
            }
        }
    }

    #[test]
    fn test_node_with_custom_resource_limits() {
        let script = create_test_script("// empty");
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(10));

        let node = Node::with_resource_limits(script, limits);
        assert!(node.is_ok());

        let node = node.unwrap();
        assert_eq!(node.resource_limits().execution_timeout.as_secs(), 10);
    }

    #[test]
    fn test_node_default_resource_limits() {
        let script = create_test_script("// empty");
        let node = Node::new(script).unwrap();

        assert_eq!(node.resource_limits().execution_timeout.as_secs(), 30);
    }

    #[test]
    fn test_node_with_invalid_resource_limits_zero_timeout() {
        let script = create_test_script("// empty");
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::ZERO);

        let result = Node::with_resource_limits(script, limits);
        assert!(result.is_err());
        if let Err(e) = result {
            let err = e.to_string();
            assert!(err.contains("Invalid resource limits"));
            assert!(err.contains("greater than zero"));
        }
    }

    #[test]
    fn test_node_with_invalid_resource_limits_excessive_timeout() {
        let script = create_test_script("// empty");
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(7200)); // 2 hours

        let result = Node::with_resource_limits(script, limits);
        assert!(result.is_err());
        if let Err(e) = result {
            let err = e.to_string();
            assert!(err.contains("Invalid resource limits"));
            assert!(err.contains("1 hour"));
        }
    }

    #[test]
    fn test_node_builder_pattern_resource_limits() {
        let script = create_test_script("// empty");
        let node = Node::new(script).unwrap()
            .and_resource_limits(ResourceLimits::new()
                .with_execution_timeout(Duration::from_secs(15)));

        assert_eq!(node.resource_limits().execution_timeout.as_secs(), 15);
    }

    #[tokio::test]
    async fn test_execution_timeout_enforcement() {
        let script = create_test_script(r#"
            madrpc.register('hang', function() {
                // Busy wait for longer than the timeout
                const start = Date.now();
                while (Date.now() - start < 10000) {
                    // Busy wait for 10 seconds
                }
                return { done: true };
            });
        "#);

        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_millis(500)); // 500ms timeout

        let node = Node::with_resource_limits(script, limits).unwrap();

        let start = std::time::Instant::now();
        let result = node.call_rpc("hang", json!({})).await;
        let elapsed = start.elapsed();

        // Should timeout with an error
        assert!(result.is_err(), "RPC call should timeout");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeded time limit") || err.contains("timeout"),
            "Error should mention timeout: {}", err);

        // Verify it timed out in approximately the right amount of time
        // (with generous tolerance for test execution time)
        assert!(elapsed.as_millis() >= 400, "Should take at least 400ms, took {}ms", elapsed.as_millis());
        assert!(elapsed.as_millis() <= 1500, "Should take at most 1500ms, took {}ms", elapsed.as_millis());
    }

    #[tokio::test]
    async fn test_execution_timeout_allows_fast_operations() {
        let script = create_test_script(r#"
            madrpc.register('fast', function(args) {
                return { result: args.x * 2 };
            });
        "#);

        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_millis(5000));

        let node = Node::with_resource_limits(script, limits).unwrap();

        let result = node.call_rpc("fast", json!({"x": 21})).await;

        // Should succeed quickly
        assert!(result.is_ok(), "RPC call should succeed");
        assert_eq!(result.unwrap(), json!({"result": 42}));
    }
}
