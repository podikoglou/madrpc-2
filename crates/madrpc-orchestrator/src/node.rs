use std::time::Instant;

/// Reason why a node is disabled
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableReason {
    /// Manually disabled by user - should never be auto-re-enabled
    Manual,
    /// Automatically disabled due to health check failures - can be auto-re-enabled
    HealthCheck,
}

/// Result of a health check
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthCheckStatus {
    Healthy,
    Unhealthy(String),
}

/// A node in the load balancer with its state
#[derive(Debug, Clone)]
pub struct Node {
    pub addr: String,
    pub enabled: bool,
    pub disable_reason: Option<DisableReason>,
    pub consecutive_failures: u32,
    pub last_health_check: Option<Instant>,
    pub last_health_check_status: Option<HealthCheckStatus>,
}

impl Node {
    pub fn new(addr: String) -> Self {
        Self {
            addr,
            enabled: true,
            disable_reason: None,
            consecutive_failures: 0,
            last_health_check: None,
            last_health_check_status: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_creation() {
        let node = Node::new("localhost:9001".to_string());
        assert_eq!(node.addr, "localhost:9001");
        assert!(node.enabled);
        assert!(node.disable_reason.is_none());
        assert_eq!(node.consecutive_failures, 0);
        assert!(node.last_health_check.is_none());
        assert!(node.last_health_check_status.is_none());
    }

    #[test]
    fn test_disable_reason_equality() {
        assert_eq!(DisableReason::Manual, DisableReason::Manual);
        assert_eq!(DisableReason::HealthCheck, DisableReason::HealthCheck);
        assert_ne!(DisableReason::Manual, DisableReason::HealthCheck);
    }

    #[test]
    fn test_health_status_equality() {
        assert_eq!(
            HealthCheckStatus::Healthy,
            HealthCheckStatus::Healthy
        );
        assert_eq!(
            HealthCheckStatus::Unhealthy("error".to_string()),
            HealthCheckStatus::Unhealthy("error".to_string())
        );
        assert_ne!(
            HealthCheckStatus::Healthy,
            HealthCheckStatus::Unhealthy("error".to_string())
        );
    }
}
