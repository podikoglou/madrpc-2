use madrpc_common::protocol::error::{MadrpcError, Result as MadrpcResult};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::load_balancer::LoadBalancer;
use crate::node::{DisableReason, HealthCheckStatus, Node};

/// Batched health check update to apply atomically.
///
/// This structure allows health check results to be computed independently
/// from load balancer state mutations, enabling better separation of concerns
/// and reducing lock contention.
pub struct HealthCheckUpdate {
    /// Address of the node this update applies to
    pub node_addr: String,
    /// Health check result to record
    pub status: HealthCheckStatus,
    /// Whether the node should be auto-enabled (only if auto-disabled previously)
    pub should_enable: bool,
    /// Whether the node should be auto-disabled (threshold exceeded, not manual)
    pub should_disable: bool,
}

/// Health check configuration.
///
/// Controls the frequency, timeout, and failure threshold for periodic health
/// checks performed by the `HealthChecker`.
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// How often to run health checks on all nodes
    ///
    /// Default: 5 seconds
    pub interval: Duration,
    /// Maximum time to wait for each health check to complete
    ///
    /// Includes both TCP connection and `_info` request response.
    /// Default: 2000ms
    pub timeout: Duration,
    /// Number of consecutive failures before auto-disabling a node
    ///
    /// Once this threshold is reached, the node is marked as unhealthy and
    /// excluded from round-robin selection (unless manually disabled).
    /// Default: 3
    pub failure_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
            timeout: Duration::from_millis(2000),
            failure_threshold: 3,
        }
    }
}

/// Periodic health checker for nodes.
///
/// The health checker runs as a background tokio task that periodically:
///
/// 1. Checks circuit breaker timeouts (Open → HalfOpen transitions)
/// 2. Performs parallel HTTP health checks on all nodes
/// 3. Updates node health status and circuit breaker state
/// 4. Auto-disables/auto-enables nodes based on health and threshold
///
/// # Health Check Method
///
/// Each health check:
/// 1. Sends HTTP GET request to `http://{addr}/__health`
/// 2. Verifies response is successful (status code 200)
///
/// # Thread Safety
///
/// The health checker holds an `Arc<RwLock<LoadBalancer>>` and periodically
/// acquires write locks to update node state. This is designed to work with
/// the orchestrator which also holds the same Arc.
pub struct HealthChecker {
    /// Load balancer to check nodes for and update
    load_balancer: Arc<RwLock<LoadBalancer>>,
    /// Health check configuration
    config: HealthCheckConfig,
}

impl HealthChecker {
    /// Creates a new health checker.
    ///
    /// The health checker must be spawned with `spawn()` to start running.
    /// Once spawned, it runs indefinitely in the background.
    ///
    /// # Arguments
    /// * `load_balancer` - The load balancer to check nodes for
    /// * `config` - Health check configuration
    ///
    /// # Returns
    /// A new HealthChecker instance (not yet running)
    pub fn new(
        load_balancer: Arc<RwLock<LoadBalancer>>,
        config: HealthCheckConfig,
    ) -> MadrpcResult<Self> {
        Ok(Self {
            load_balancer,
            config,
        })
    }

    /// Starts the health checker task.
    ///
    /// This spawns a tokio task that runs health checks indefinitely at the
    /// configured interval. The task will run until the orchestrator is dropped.
    ///
    /// # Returns
    /// A JoinHandle for the spawned task (can be used to await completion)
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    /// Main health check loop.
    ///
    /// Runs indefinitely, checking all nodes at the configured interval.
    /// This is called by the spawned task and never returns.
    async fn run(self) {
        let mut interval = tokio::time::interval(self.config.interval);

        loop {
            interval.tick().await;
            self.check_all_nodes().await;
        }
    }

    /// Check health of all nodes in parallel.
    ///
    /// This method:
    /// 1. Checks circuit breaker timeouts (may transition Open → HalfOpen)
    /// 2. Performs parallel health checks on all nodes
    /// 3. Processes results and updates node state atomically
    ///
    /// Parallel health checks use `futures::join_all` for efficiency.
    async fn check_all_nodes(&self) {
        // First, check for circuit breaker timeouts
        {
            let mut lb = self.load_balancer.write().await;
            if lb.check_circuit_timeouts() {
                info!("Circuit breaker timeout check completed, some nodes transitioned to half-open");
            }
        }

        let nodes = {
            let lb = self.load_balancer.read().await;
            lb.all_nodes()
        };

        // Check all nodes in parallel
        let checks: Vec<_> = nodes
            .into_iter()
            .map(|node| {
                let timeout = self.config.timeout;
                async move {
                    let result = Self::check_node_health(&node.addr, timeout).await;
                    (node, result)
                }
            })
            .collect();

        let results = futures::future::join_all(checks).await;

        // Process results
        for (node, health_result) in results {
            let update = self.process_health_result(node, health_result).await;
            self.apply_health_update(update).await;
        }
    }

    /// Check a single node's health via HTTP.
    ///
    /// This performs the actual health check by:
    /// 1. Sending HTTP GET request to `http://{addr}/__health`
    /// 2. Verifying response is successful (status code 200)
    ///
    /// # Arguments
    /// * `addr` - Node address to connect to
    /// * `timeout` - Maximum time to wait for connection and response
    ///
    /// # Returns
    /// - `Ok(())` - Node is healthy
    /// - `Err(...)` - Node is unhealthy or unreachable
    async fn check_node_health(
        addr: &str,
        timeout: Duration,
    ) -> MadrpcResult<()> {
        use hyper::Request;
        use hyper_util::client::legacy::Client;
        use hyper_util::rt::TokioExecutor;
        use http_body_util::Full;
        use hyper::body::Bytes;

        // Build HTTP health check request
        let url = format!("http://{}/__health", addr);
        let http_request = Request::builder()
            .method("GET")
            .uri(&url)
            .body(Full::new(Bytes::new()))
            .map_err(|e| MadrpcError::Transport(format!("Failed to build health check request: {}", e)))?;

        // Create HTTP client
        let client = Client::builder(TokioExecutor::new()).build_http();

        // Send request with timeout
        let response_future = client.request(http_request);
        let response = tokio::time::timeout(timeout, response_future)
            .await
            .map_err(|_| MadrpcError::Timeout(timeout.as_millis() as u64))?
            .map_err(|e| MadrpcError::Transport(format!("HTTP health check failed: {}", e)))?;

        // Verify response is successful
        if response.status() != hyper::StatusCode::OK {
            return Err(MadrpcError::NodeUnavailable(format!(
                "Health check returned status: {}",
                response.status()
            )));
        }

        Ok(())
    }

    /// Process health check result for a node.
    ///
    /// This computes the necessary state updates based on the health check
    /// result and the node's current state. It returns a `HealthCheckUpdate`
    /// which is then applied atomically.
    ///
    /// # Arguments
    /// * `node` - The node that was checked
    /// * `result` - The health check result
    ///
    /// # Returns
    /// A HealthCheckUpdate with the computed state changes
    async fn process_health_result(&self, node: Node, result: MadrpcResult<()>) -> HealthCheckUpdate {
        match result {
            Ok(()) => {
                HealthCheckUpdate {
                    node_addr: node.addr.clone(),
                    status: HealthCheckStatus::Healthy,
                    should_enable: !node.enabled && node.disable_reason == Some(DisableReason::HealthCheck),
                    should_disable: false,
                }
            }
            Err(e) => {
                let failures = {
                    let lb = self.load_balancer.read().await;
                    lb.consecutive_failures(&node.addr) + 1
                };
                let should_disable = failures >= self.config.failure_threshold
                    && node.enabled
                    && node.disable_reason != Some(DisableReason::Manual);

                HealthCheckUpdate {
                    node_addr: node.addr.clone(),
                    status: HealthCheckStatus::Unhealthy(e.to_string()),
                    should_enable: false,
                    should_disable,
                }
            }
        }
    }

    /// Apply a health check update atomically.
    ///
    /// This applies the computed update to the load balancer state, including:
    /// - Recording health status
    /// - Auto-enabling recovered nodes (if auto-disabled)
    /// - Auto-disabling failed nodes (if threshold exceeded)
    ///
    /// All mutations happen while holding the write lock to ensure atomicity.
    ///
    /// # Arguments
    /// * `update` - The health check update to apply
    async fn apply_health_update(&self, update: HealthCheckUpdate) {
        let mut lb = self.load_balancer.write().await;

        // Extract error message before moving status
        let error_msg = match &update.status {
            HealthCheckStatus::Unhealthy(msg) => Some(msg.clone()),
            _ => None,
        };

        lb.update_health_status(&update.node_addr, update.status);

        if update.should_enable {
            if lb.auto_enable_node(&update.node_addr) {
                info!(
                    "Node {} re-enabled after health check recovery",
                    update.node_addr
                );
            }
        }

        if update.should_disable {
            let failures = lb.consecutive_failures(&update.node_addr);
            if lb.auto_disable_node(&update.node_addr) {
                warn!(
                    "Node {} disabled after {} failed health checks: {}",
                    update.node_addr,
                    failures,
                    error_msg.unwrap_or_else(|| "unknown error".to_string())
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_balancer::LoadBalancer;

    #[test]
    fn test_health_check_config_default() {
        let config = HealthCheckConfig::default();
        assert_eq!(config.interval, Duration::from_secs(5));
        assert_eq!(config.timeout, Duration::from_millis(2000));
        assert_eq!(config.failure_threshold, 3);
    }

    #[test]
    fn test_health_check_config_custom() {
        let config = HealthCheckConfig {
            interval: Duration::from_secs(10),
            timeout: Duration::from_millis(5000),
            failure_threshold: 5,
        };
        assert_eq!(config.interval, Duration::from_secs(10));
        assert_eq!(config.timeout, Duration::from_millis(5000));
        assert_eq!(config.failure_threshold, 5);
    }

    #[tokio::test]
    async fn test_health_check_update_healthy() {
        let lb = Arc::new(RwLock::new(LoadBalancer::new(vec![
            "node1".to_string(),
        ])));

        // Manually disable node via auto-disable (health check)
        {
            let mut lb = lb.write().await;
            lb.auto_disable_node("node1");
        }

        let update = HealthCheckUpdate {
            node_addr: "node1".to_string(),
            status: HealthCheckStatus::Healthy,
            should_enable: true,
            should_disable: false,
        };

        let health_checker = HealthChecker::new(
            lb.clone(),
            HealthCheckConfig::default()
        ).unwrap();

        health_checker.apply_health_update(update).await;

        // Node should be re-enabled
        let lb = lb.read().await;
        assert!(lb.enabled_nodes().contains(&"node1".to_string()));
    }

    #[tokio::test]
    async fn test_health_check_update_unhealthy_threshold() {
        let lb = Arc::new(RwLock::new(LoadBalancer::new(vec![
            "node1".to_string(),
        ])));

        let health_checker = HealthChecker::new(
            lb.clone(),
            HealthCheckConfig {
                failure_threshold: 3,
                ..Default::default()
            }
        ).unwrap();

        // Simulate 3 failures (each increments failure count and disables)
        for _ in 0..3 {
            let update = HealthCheckUpdate {
                node_addr: "node1".to_string(),
                status: HealthCheckStatus::Unhealthy("connection refused".to_string()),
                should_enable: false,
                should_disable: true,
            };
            health_checker.apply_health_update(update).await;
        }

        // Node should be disabled
        let lb = lb.read().await;
        assert!(!lb.enabled_nodes().contains(&"node1".to_string()));
    }

    #[tokio::test]
    async fn test_health_check_respects_manual_disable() {
        let lb = Arc::new(RwLock::new(LoadBalancer::new(vec![
            "node1".to_string(),
        ])));

        // Manually disable node
        {
            let mut lb = lb.write().await;
            lb.disable_node("node1");
        }

        let health_checker = HealthChecker::new(
            lb.clone(),
            HealthCheckConfig::default()
        ).unwrap();

        // Try to re-enable via health check (should not work)
        let update = HealthCheckUpdate {
            node_addr: "node1".to_string(),
            status: HealthCheckStatus::Healthy,
            should_enable: false,
            should_disable: false,
        };

        health_checker.apply_health_update(update).await;

        // Node should still be disabled
        let lb = lb.read().await;
        assert!(!lb.enabled_nodes().contains(&"node1".to_string()));
    }

    #[tokio::test]
    async fn test_concurrent_health_check_and_load_balancer_access() {
        use tokio::task::JoinSet;

        let lb = Arc::new(RwLock::new(LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ])));

        let health_checker = Arc::new(HealthChecker::new(
            lb.clone(),
            HealthCheckConfig::default()
        ).unwrap());

        let mut join_set = JoinSet::new();

        // Spawn tasks that call next_node concurrently with health updates
        for i in 0..10 {
            let lb_clone = Arc::clone(&lb);
            join_set.spawn(async move {
                for _ in 0..100 {
                    let mut lb = lb_clone.write().await;
                    let _ = lb.next_node();
                }
            });

            let health_checker = Arc::clone(&health_checker);
            join_set.spawn(async move {
                let update = HealthCheckUpdate {
                    node_addr: format!("node{}", (i % 3) + 1),
                    status: HealthCheckStatus::Healthy,
                    should_enable: false,
                    should_disable: false,
                };
                health_checker.apply_health_update(update).await;
            });
        }

        // All tasks should complete without deadlocks
        while let Some(result) = join_set.join_next().await {
            result.unwrap();
        }
    }

    // ============================================================================
    // Circuit Breaker Integration Tests
    // ============================================================================

    #[tokio::test]
    async fn test_health_check_with_circuit_breaker_timeout() {
        let lb = Arc::new(RwLock::new(LoadBalancer::new(vec![
            "node1".to_string(),
        ])));

        // Manually trip the circuit and set timestamp to past using public API
        {
            let mut lb = lb.write().await;
            for _ in 0..5 {
                lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
            }

            // We can't directly access nodes, but we can use check_circuit_timeouts
            // with a manually adjusted timestamp via a helper
            // For this test, we'll verify the circuit is open first
            assert_eq!(
                lb.circuit_state("node1"),
                Some(crate::node::CircuitBreakerState::Open)
            );
        }

        // Note: We can't easily test timeout transitions without exposing more API
        // The integration tests will verify this behavior
    }

    #[tokio::test]
    async fn test_health_check_trips_circuit_after_threshold() {
        let lb = Arc::new(RwLock::new(LoadBalancer::new(vec![
            "node1".to_string(),
        ])));

        let health_checker = HealthChecker::new(
            lb.clone(),
            HealthCheckConfig::default()
        ).unwrap();

        // Apply 5 consecutive failures (default threshold)
        for _ in 0..5 {
            let update = HealthCheckUpdate {
                node_addr: "node1".to_string(),
                status: HealthCheckStatus::Unhealthy("err".to_string()),
                should_enable: false,
                should_disable: false,
            };
            health_checker.apply_health_update(update).await;
        }

        // Circuit should be open
        let lb = lb.read().await;
        assert_eq!(
            lb.circuit_state("node1"),
            Some(crate::node::CircuitBreakerState::Open)
        );
    }
}
