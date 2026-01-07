use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::{MadrpcError, Result};
use madrpc_common::transport::TcpTransportAsync;
use madrpc_metrics::{MetricsCollector, OrchestratorMetricsCollector};
use crate::load_balancer::LoadBalancer;
use crate::node::Node;
use crate::health_checker::{HealthChecker, HealthCheckConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::info;

/// Configuration for retry logic with exponential backoff.
///
/// When a request fails due to transient errors (connection issues, timeouts),
/// the orchestrator will retry with exponential backoff up to `max_retries` times.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (excluding initial attempt)
    ///
    /// Total attempts = max_retries + 1
    /// Default: 3
    pub max_retries: usize,
    /// Initial backoff in milliseconds
    ///
    /// First retry waits this long, subsequent retries use exponential backoff.
    /// Default: 50ms
    pub initial_backoff_ms: u64,
    /// Maximum backoff in milliseconds
    ///
    /// Exponential backoff is capped at this value.
    /// Default: 5000ms (5 seconds)
    pub max_backoff_ms: u64,
    /// Exponential backoff multiplier
    ///
    /// Each retry waits: previous_backoff * multiplier
    /// Default: 2.0 (doubles each time)
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 50,
            max_backoff_ms: 5000,
            backoff_multiplier: 2.0,
        }
    }
}

/// MaDRPC Orchestrator - "stupid" forwarder with load balancing.
///
/// The orchestrator is the central component that sits between clients and
/// compute nodes. It receives requests from clients, uses round-robin load
/// balancing with circuit breaking to select a node, forwards the request,
/// and returns the response.
///
/// # Design Philosophy
///
/// The orchestrator is intentionally "stupid" - it does NOT execute JavaScript
/// code or have a Boa engine. Its only responsibilities are:
///
/// 1. **Load Balancing**: Distribute requests across nodes via round-robin
/// 2. **Circuit Breaking**: Skip unhealthy nodes to prevent cascading failures
/// 3. **Health Checking**: Periodically verify node availability
/// 4. **Request Forwarding**: Forward requests and return responses
/// 5. **Retry Logic**: Retry failed requests with exponential backoff
///
/// # Connection-per-Request Strategy
///
/// The orchestrator creates a new TCP connection for each request rather than
/// maintaining a connection pool. This design choice enables:
///
/// - **True Parallelism**: Multiple requests to the same node execute concurrently
/// - **Simplified State Management**: No need to manage shared connection lifecycles
/// - **Fault Isolation**: Connection failures don't affect other requests
///
/// The overhead of connection creation is acceptable because the orchestrator
/// typically runs in the same network as nodes (low latency).
///
/// # Metrics and Monitoring
///
/// The orchestrator collects metrics for:
/// - Requests per node
/// - Method call latency and success rate
/// - Built-in `_metrics` and `_info` endpoints
pub struct Orchestrator {
    /// Thread-safe load balancer with circuit breaker
    load_balancer: Arc<RwLock<LoadBalancer>>,
    /// Async TCP transport for connections to nodes
    transport: TcpTransportAsync,
    /// Metrics collector for monitoring
    metrics_collector: Arc<OrchestratorMetricsCollector>,
    /// Retry configuration for failed requests
    retry_config: RetryConfig,
    /// Background health checker task handle (kept to prevent task from being dropped)
    _health_checker_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Orchestrator {
    /// Creates a new orchestrator with static node list and default configs.
    ///
    /// This is the simplest way to create an orchestrator. It uses default
    /// health check (5s interval, 2s timeout, 3 failures threshold) and
    /// retry (3 attempts, 50ms initial backoff, 2x multiplier) configs.
    ///
    /// # Arguments
    /// * `node_addrs` - List of node addresses (e.g., "127.0.0.1:9001")
    ///
    /// # Returns
    /// A new Orchestrator instance with health checker running in background
    ///
    /// # Example
    /// ```no_run
    /// # use madrpc_orchestrator::Orchestrator;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let orchestrator = Orchestrator::new(vec![
    ///     "127.0.0.1:9001".to_string(),
    ///     "127.0.0.1:9002".to_string(),
    /// ]).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(node_addrs: Vec<String>) -> Result<Self> {
        Self::with_retry_config(
            node_addrs,
            HealthCheckConfig::default(),
            RetryConfig::default(),
        ).await
    }

    /// Creates a new orchestrator with static node list and custom health check config.
    ///
    /// Use this when you need non-default health check behavior. Uses default
    /// retry config.
    ///
    /// # Arguments
    /// * `node_addrs` - List of node addresses
    /// * `health_config` - Health check configuration
    pub async fn with_config(
        node_addrs: Vec<String>,
        health_config: HealthCheckConfig,
    ) -> Result<Self> {
        Self::with_retry_config(
            node_addrs,
            health_config,
            RetryConfig::default(),
        ).await
    }

    /// Creates a new orchestrator with static node list and custom configs.
    ///
    /// This is the most flexible constructor, allowing customization of both
    /// health checking and retry behavior.
    ///
    /// # Arguments
    /// * `node_addrs` - List of node addresses
    /// * `health_config` - Health check configuration
    /// * `retry_config` - Retry configuration
    ///
    /// # Returns
    /// A new Orchestrator instance with health checker running in background
    ///
    /// # Behavior
    /// - Spawns background health checker task
    /// - Initializes metrics collector
    /// - All nodes start enabled with closed circuit breakers
    pub async fn with_retry_config(
        node_addrs: Vec<String>,
        health_config: HealthCheckConfig,
        retry_config: RetryConfig,
    ) -> Result<Self> {
        let load_balancer = Arc::new(RwLock::new(LoadBalancer::new(node_addrs)));
        let transport = TcpTransportAsync::new()?;

        // Initialize metrics
        let metrics_collector = Arc::new(OrchestratorMetricsCollector::new());

        // Spawn health checker
        let health_checker = HealthChecker::new(load_balancer.clone(), health_config)?;
        let health_checker_handle = health_checker.spawn();

        info!("Orchestrator initialized with health checking and retry logic");

        Ok(Self {
            load_balancer,
            transport,
            metrics_collector,
            retry_config,
            _health_checker_handle: Some(health_checker_handle),
        })
    }

    /// Forwards a request to the next available node with retry logic.
    ///
    /// This is the main entry point for handling client requests. It implements:
    ///
    /// 1. **Built-in methods**: Handles `_metrics` and `_info` requests directly
    /// 2. **Load balancing**: Selects next node via round-robin
    /// 3. **Circuit breaking**: Skips nodes with Open circuits
    /// 4. **Retry logic**: Retries failed requests with exponential backoff
    /// 5. **Connection-per-request**: Creates fresh connection for each request
    /// 6. **Metrics collection**: Records request latency and success rate
    ///
    /// # Connection-per-Request Strategy
    ///
    /// Each request creates its own TCP connection to avoid serialization through
    /// shared Arc<Mutex<TcpStream>>. This enables true parallelism when multiple
    /// requests target the same node.
    ///
    /// # Retry Logic
    ///
    /// Retryable errors (connection failures, timeouts, node unavailable) trigger
    /// retry with exponential backoff:
    ///
    /// - Attempt 0: Initial request
    /// - Attempt 1: Wait `initial_backoff_ms` (default: 50ms)
    /// - Attempt 2: Wait `initial_backoff_ms * multiplier` (default: 100ms)
    /// - Attempt 3: Wait `previous * multiplier` (default: 200ms)
    /// - Capped at `max_backoff_ms` (default: 5000ms)
    ///
    /// # Arguments
    /// * `request` - The request to forward
    ///
    /// # Returns
    /// - `Ok(response)` - Successful response from node
    /// - `Err(MadrpcError::AllNodesFailed)` - All retry attempts exhausted
    /// - `Err(...)` - Non-retryable error (e.g., invalid request)
    ///
    /// # Example
    /// ```no_run
    /// # use madrpc_orchestrator::Orchestrator;
    /// # use madrpc_common::protocol::Request;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let orchestrator = Orchestrator::new(vec![]).await?;
    /// use serde_json::json;
    ///
    /// let request = Request::new("my_method", json!({"arg": 42}));
    /// let response = orchestrator.forward_request(&request).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn forward_request(&self, request: &Request) -> Result<Response> {
        // Check for metrics/info requests (do NOT forward these)
        if self
            .metrics_collector
            .is_metrics_request(&request.method)
        {
            return self
                .metrics_collector
                .handle_metrics_request(&request.method, request.id);
        }

        let start_time = Instant::now();
        let method = request.method.clone();
        let mut backoff_ms = self.retry_config.initial_backoff_ms;

        // Retry loop with exponential backoff
        for attempt in 0..=self.retry_config.max_retries {
            // Get next node via round-robin
            let node_addr = {
                let mut lb = self.load_balancer.write().await;
                lb.next_node().ok_or_else(|| {
                    if attempt < self.retry_config.max_retries {
                        MadrpcError::NodeUnavailable(
                            format!("No available nodes (attempt {}/{})",
                                attempt + 1, self.retry_config.max_retries + 1)
                        )
                    } else {
                        MadrpcError::AllNodesFailed
                    }
                })?
            };

            // Track which node received the request
            self.metrics_collector.record_node_request(&node_addr);

            // Try to connect to the node
            let mut stream = match self.transport.connect(&node_addr).await {
                Ok(s) => s,
                Err(e) if attempt < self.retry_config.max_retries => {
                    tracing::warn!(
                        "Connection to {} failed (attempt {}): {}, retrying in {}ms",
                        node_addr, attempt + 1, e, backoff_ms
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = std::cmp::min(
                        (backoff_ms as f64 * self.retry_config.backoff_multiplier) as u64,
                        self.retry_config.max_backoff_ms
                    );
                    continue;
                }
                Err(e) => return Err(e),
            };

            // Try to send the request
            let response = self.transport.send_request(&mut stream, request).await;
            let success = response.is_ok();

            // If request failed and we have retries left, check if it's retryable
            if let Err(ref e) = response {
                if attempt < self.retry_config.max_retries && self.is_retryable(e) {
                    tracing::warn!(
                        "Request to {} failed (attempt {}): {}, retrying in {}ms",
                        node_addr, attempt + 1, e, backoff_ms
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = std::cmp::min(
                        (backoff_ms as f64 * self.retry_config.backoff_multiplier) as u64,
                        self.retry_config.max_backoff_ms
                    );
                    continue;
                }
            }

            // Connection is closed here when stream is dropped

            // Record metrics based on response
            let response = response?;
            self.metrics_collector.record_call(&method, start_time, success);
            return Ok(response);
        }

        // This should never be reached since we either return Ok or break with Err
        unreachable!("Retry loop should always return or error")
    }

    /// Checks if an error is retryable.
    ///
    /// Retryable errors are transient failures that may succeed on retry:
    /// - `AllNodesFailed`: All nodes unavailable
    /// - `NodeUnavailable`: Specific node unavailable
    ///
    /// Non-retryable errors are permanent failures:
    /// - `InvalidRequest`: Malformed request
    /// - `JavaScriptExecution`: Script execution failed
    /// - `Transport`: Other transport errors
    ///
    /// # Arguments
    /// * `error` - The error to check
    ///
    /// # Returns
    /// `true` if the error is retryable
    fn is_retryable(&self, error: &MadrpcError) -> bool {
        Self::is_retryable_static(error)
    }

    /// Static method to check if an error is retryable (for testing).
    ///
    /// This is a public static version of `is_retryable` for use in tests.
    ///
    /// # Arguments
    /// * `error` - The error to check
    ///
    /// # Returns
    /// `true` if the error is retryable
    pub fn is_retryable_static(error: &MadrpcError) -> bool {
        matches!(error, MadrpcError::AllNodesFailed | MadrpcError::NodeUnavailable(_))
    }

    /// Adds a node to the load balancer.
    ///
    /// New nodes are enabled by default with closed circuit breakers.
    /// Duplicate nodes are ignored (no-op).
    ///
    /// # Arguments
    /// * `node_addr` - The node address to add (e.g., "127.0.0.1:9003")
    pub async fn add_node(&self, node_addr: String) {
        let mut lb = self.load_balancer.write().await;
        lb.add_node(node_addr);
    }

    /// Removes a node from the load balancer.
    ///
    /// Note: Active connections to this node will complete and close naturally.
    /// New requests will no longer be forwarded to this node.
    ///
    /// # Arguments
    /// * `node_addr` - The node address to remove
    pub async fn remove_node(&self, node_addr: &str) {
        let mut lb = self.load_balancer.write().await;
        lb.remove_node(node_addr);
    }

    /// Manually disables a node.
    ///
    /// Manually disabled nodes are marked with `DisableReason::Manual` and will
    /// never be auto-re-enabled by the health checker. They must be manually
    /// re-enabled via `enable_node()`.
    ///
    /// # Arguments
    /// * `node_addr` - The node address to disable
    ///
    /// # Returns
    /// - `true` - Node was found and disabled
    /// - `false` - Node was not found
    pub async fn disable_node(&self, node_addr: &str) -> bool {
        let mut lb = self.load_balancer.write().await;
        let disabled = lb.disable_node(node_addr);
        if disabled {
            info!("Manually disabled node: {}", node_addr);
        }
        disabled
    }

    /// Manually enables a node.
    ///
    /// This resets the node's disable reason, consecutive failures, and adds it
    /// back to the enabled nodes list.
    ///
    /// # Arguments
    /// * `node_addr` - The node address to enable
    ///
    /// # Returns
    /// - `true` - Node was found and enabled
    /// - `false` - Node was not found
    pub async fn enable_node(&self, node_addr: &str) -> bool {
        let mut lb = self.load_balancer.write().await;
        let enabled = lb.enable_node(node_addr);
        if enabled {
            info!("Manually enabled node: {}", node_addr);
        }
        enabled
    }

    /// Gets all nodes with their status.
    ///
    /// This returns cloned Node instances, which include all state information
    /// (health status, circuit state, consecutive failures, etc.).
    ///
    /// # Returns
    /// Vector of all nodes with their current state
    pub async fn nodes_with_status(&self) -> Vec<Node> {
        let lb = self.load_balancer.read().await;
        lb.all_nodes()
    }

    /// Gets the number of nodes (including disabled ones).
    ///
    /// # Returns
    /// Total number of nodes in the load balancer
    pub async fn node_count(&self) -> usize {
        let lb = self.load_balancer.read().await;
        lb.node_count()
    }

    /// Gets list of all node addresses (including disabled ones).
    ///
    /// This is useful for display and debugging purposes.
    ///
    /// # Returns
    /// Vector of all node addresses
    pub async fn nodes(&self) -> Vec<String> {
        let lb = self.load_balancer.read().await;
        lb.nodes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full tests require running nodes
    // These are basic unit tests

    #[tokio::test]
    async fn test_orchestrator_creation() {
        let nodes = vec!["localhost:9001".to_string(), "localhost:9002".to_string()];
        let orch = Orchestrator::new(nodes).await;
        assert!(orch.is_ok());
    }

    #[tokio::test]
    async fn test_orchestrator_node_count() {
        let nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let orch = Orchestrator::new(nodes).await.unwrap();
        assert_eq!(orch.node_count().await, 3);
    }

    #[tokio::test]
    async fn test_orchestrator_add_node() {
        let orch = Orchestrator::new(vec![]).await.unwrap();
        orch.add_node("new-node".to_string()).await;
        assert_eq!(orch.node_count().await, 1);
    }

    #[tokio::test]
    async fn test_orchestrator_remove_node() {
        let orch = Orchestrator::new(vec!["node1".to_string()])
            .await
            .unwrap();
        orch.remove_node("node1").await;
        assert_eq!(orch.node_count().await, 0);
    }

    #[tokio::test]
    async fn test_orchestrator_nodes() {
        let nodes = vec!["node1".to_string(), "node2".to_string()];
        let orch = Orchestrator::new(nodes.clone()).await.unwrap();
        let mut result = orch.nodes().await;
        result.sort(); // HashMap doesn't guarantee order
        assert_eq!(result, nodes);
    }

    #[tokio::test]
    async fn test_orchestrator_add_duplicate_node() {
        let orch = Orchestrator::new(vec!["node1".to_string()])
            .await
            .unwrap();
        orch.add_node("node1".to_string()).await;
        // duplicate should not be added
        assert_eq!(orch.node_count().await, 1);
    }

    #[tokio::test]
    async fn test_orchestrator_empty_nodes() {
        let orch = Orchestrator::new(vec![]).await.unwrap();
        assert_eq!(orch.node_count().await, 0);
        assert_eq!(orch.nodes().await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn test_orchestrator_manual_disable() {
        let orch = Orchestrator::new(vec!["node1".to_string()])
            .await
            .unwrap();
        assert!(orch.disable_node("node1").await);
        let nodes = orch.nodes_with_status().await;
        let node1 = nodes.iter().find(|n| n.addr == "node1").unwrap();
        assert!(!node1.enabled);
    }

    #[tokio::test]
    async fn test_orchestrator_manual_enable() {
        let orch = Orchestrator::new(vec!["node1".to_string()])
            .await
            .unwrap();
        orch.disable_node("node1").await;
        assert!(orch.enable_node("node1").await);
        let nodes = orch.nodes_with_status().await;
        let node1 = nodes.iter().find(|n| n.addr == "node1").unwrap();
        assert!(node1.enabled);
    }

    // ============================================================================
    // Retry Logic Tests
    // ============================================================================

    #[tokio::test]
    async fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff_ms, 50);
        assert_eq!(config.max_backoff_ms, 5000);
        assert_eq!(config.backoff_multiplier, 2.0);
    }

    #[tokio::test]
    async fn test_retry_config_custom() {
        let config = RetryConfig {
            max_retries: 5,
            initial_backoff_ms: 100,
            max_backoff_ms: 10000,
            backoff_multiplier: 3.0,
        };
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_backoff_ms, 100);
        assert_eq!(config.max_backoff_ms, 10000);
        assert_eq!(config.backoff_multiplier, 3.0);
    }

    #[test]
    fn test_exponential_backoff_calculation() {
        let config = RetryConfig::default();
        let mut backoff_ms = config.initial_backoff_ms;

        let expected = [50, 100, 200, 400, 800, 1600, 3200, 5000];

        for expected_ms in expected {
            assert_eq!(backoff_ms, expected_ms);
            backoff_ms = std::cmp::min(
                (backoff_ms as f64 * config.backoff_multiplier) as u64,
                config.max_backoff_ms
            );
        }

        // Should stay at max
        for _ in 0..5 {
            assert_eq!(backoff_ms, config.max_backoff_ms);
            backoff_ms = std::cmp::min(
                (backoff_ms as f64 * config.backoff_multiplier) as u64,
                config.max_backoff_ms
            );
        }
    }

    #[test]
    fn test_is_retryable_error() {
        // Test retryable errors
        assert!(Orchestrator::is_retryable_static(&MadrpcError::AllNodesFailed));
        assert!(Orchestrator::is_retryable_static(&MadrpcError::NodeUnavailable("test".to_string())));

        // Test non-retryable errors
        assert!(!Orchestrator::is_retryable_static(&MadrpcError::InvalidRequest("test".to_string())));
        assert!(!Orchestrator::is_retryable_static(&MadrpcError::JavaScriptExecution("test".to_string())));
        assert!(!Orchestrator::is_retryable_static(&MadrpcError::Transport("test".to_string())));
    }
}
