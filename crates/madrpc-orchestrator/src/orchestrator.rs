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

/// Configuration for retry logic with exponential backoff
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: usize,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
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

/// MaDRPC Orchestrator - "stupid" forwarder
///
/// The orchestrator receives requests from clients, uses the load balancer
/// to pick a node, forwards the request to that node, and returns the
/// node's response to the client.
///
/// IMPORTANT: The orchestrator does NOT have a Boa engine. It only forwards.
///
/// Connection strategy: Creates a new connection for each request to avoid
/// serialization through shared Arc<Mutex<TcpStream>>. This enables true
/// parallelism when multiple requests target the same node.
pub struct Orchestrator {
    load_balancer: Arc<RwLock<LoadBalancer>>,
    transport: TcpTransportAsync,
    metrics_collector: Arc<OrchestratorMetricsCollector>,
    retry_config: RetryConfig,
    _health_checker_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Orchestrator {
    /// Create a new orchestrator with static node list and default configs
    pub async fn new(node_addrs: Vec<String>) -> Result<Self> {
        Self::with_config(
            node_addrs,
            HealthCheckConfig::default(),
            RetryConfig::default(),
        ).await
    }

    /// Create a new orchestrator with static node list and custom health check config
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

    /// Create a new orchestrator with static node list and custom configs
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

    /// Forward a request to the next available node
    ///
    /// This method:
    /// 1. Gets the next node via round-robin from the load balancer
    /// 2. Creates a fresh connection to that node (no caching for true parallelism)
    /// 3. Forwards the request to the node
    /// 4. Returns the node's response
    ///
    /// Each request creates its own connection to avoid serialization through
    /// shared Arc<Mutex<TcpStream>>. This allows multiple requests to the same
    /// node to execute in parallel.
    ///
    /// Retry logic: If a request fails due to connection issues or node unavailability,
    /// the orchestrator will retry with exponential backoff up to max_retries times.
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

    /// Check if an error is retryable
    fn is_retryable(&self, error: &MadrpcError) -> bool {
        matches!(error, MadrpcError::AllNodesFailed | MadrpcError::NodeUnavailable(_))
    }

    /// Add a node to the load balancer
    pub async fn add_node(&self, node_addr: String) {
        let mut lb = self.load_balancer.write().await;
        lb.add_node(node_addr);
    }

    /// Remove a node from the load balancer
    ///
    /// Note: Active connections to this node will complete and close naturally.
    /// New requests will no longer be forwarded to this node.
    pub async fn remove_node(&self, node_addr: &str) {
        let mut lb = self.load_balancer.write().await;
        lb.remove_node(node_addr);
    }

    /// Manually disable a node
    pub async fn disable_node(&self, node_addr: &str) -> bool {
        let mut lb = self.load_balancer.write().await;
        let disabled = lb.disable_node(node_addr);
        if disabled {
            info!("Manually disabled node: {}", node_addr);
        }
        disabled
    }

    /// Manually enable a node
    pub async fn enable_node(&self, node_addr: &str) -> bool {
        let mut lb = self.load_balancer.write().await;
        let enabled = lb.enable_node(node_addr);
        if enabled {
            info!("Manually enabled node: {}", node_addr);
        }
        enabled
    }

    /// Get all nodes with their status
    pub async fn nodes_with_status(&self) -> Vec<Node> {
        let lb = self.load_balancer.read().await;
        lb.all_nodes()
    }

    /// Get the number of nodes
    pub async fn node_count(&self) -> usize {
        let lb = self.load_balancer.read().await;
        lb.node_count()
    }

    /// Get list of all nodes
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
        assert_eq!(orch.nodes().await, nodes);
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
}
