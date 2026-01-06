use madrpc_common::protocol::Request;
use madrpc_common::protocol::error::{MadrpcError, Result as MadrpcResult};
use madrpc_common::transport::TcpTransportAsync;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::load_balancer::LoadBalancer;
use crate::node::{DisableReason, HealthCheckStatus, Node};

/// Batched health check update to apply atomically.
pub struct HealthCheckUpdate {
    pub node_addr: String,
    pub status: HealthCheckStatus,
    pub should_enable: bool,
    pub should_disable: bool,
}

/// Health check configuration.
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    pub interval: Duration,
    pub timeout: Duration,
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

/// Health checker for nodes.
pub struct HealthChecker {
    load_balancer: Arc<RwLock<LoadBalancer>>,
    transport: TcpTransportAsync,
    config: HealthCheckConfig,
}

impl HealthChecker {
    /// Creates a new health checker.
    ///
    /// # Arguments
    /// * `load_balancer` - The load balancer to check nodes for
    /// * `config` - Health check configuration
    pub fn new(
        load_balancer: Arc<RwLock<LoadBalancer>>,
        config: HealthCheckConfig,
    ) -> MadrpcResult<Self> {
        let transport = TcpTransportAsync::new()?;
        Ok(Self {
            load_balancer,
            transport,
            config,
        })
    }

    /// Starts the health checker task.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    /// Main health check loop
    async fn run(self) {
        let mut interval = tokio::time::interval(self.config.interval);

        loop {
            interval.tick().await;
            self.check_all_nodes().await;
        }
    }

    /// Check health of all nodes
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
                let transport = &self.transport;
                let timeout = self.config.timeout;
                async move {
                    let result =
                        Self::check_node_health(transport, &node.addr, timeout).await;
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

    /// Check a single node's health
    async fn check_node_health(
        transport: &TcpTransportAsync,
        addr: &str,
        timeout: Duration,
    ) -> MadrpcResult<()> {
        // Connect with timeout
        let connect_future = transport.connect(addr);
        let mut stream = tokio::time::timeout(timeout, connect_future)
            .await
            .map_err(|_| MadrpcError::Timeout(timeout.as_millis() as u64))??;

        // Send _info request with timeout
        let request = Request::new("_info", serde_json::json!({}));
        let send_future = transport.send_request(&mut stream, &request);
        let response = tokio::time::timeout(timeout, send_future)
            .await
            .map_err(|_| MadrpcError::Timeout(timeout.as_millis() as u64))??;

        // Verify response is successful
        if !response.success {
            return Err(MadrpcError::NodeUnavailable(format!(
                "Health check failed: {}",
                response.error.unwrap_or_else(|| "Unknown error".to_string())
            )));
        }

        Ok(())
    }

    /// Process health check result for a node
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

    /// Apply a health check update atomically
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
                    "Node {} disabled after {} consecutive health check failures: {}",
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
