use madrpc_common::protocol::Request;
use madrpc_common::protocol::error::{MadrpcError, Result as MadrpcResult};
use madrpc_common::transport::TcpTransportAsync;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::load_balancer::LoadBalancer;
use crate::node::{DisableReason, HealthCheckStatus, Node};

/// Batched health check update to apply atomically
struct HealthCheckUpdate {
    node_addr: String,
    status: HealthCheckStatus,
    should_enable: bool,
    should_disable: bool,
}

/// Health check configuration
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

/// Health checker for nodes
pub struct HealthChecker {
    load_balancer: Arc<RwLock<LoadBalancer>>,
    transport: TcpTransportAsync,
    config: HealthCheckConfig,
}

impl HealthChecker {
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

    /// Start the health checker task
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
}
