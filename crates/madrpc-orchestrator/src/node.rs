use std::time::{Instant, SystemTime};

/// Reason why a node is disabled.
///
/// This distinction is important because manually disabled nodes should never
/// be auto-re-enabled by the health checker, while auto-disabled nodes can be
/// re-enabled when they recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableReason {
    /// Manually disabled by user via API - should never be auto-re-enabled
    Manual,
    /// Automatically disabled due to health check failures - can be auto-re-enabled on recovery
    HealthCheck,
}

/// Result of a health check performed on a node.
///
/// Used by the health checker to update node state and by circuit breaker logic
/// to determine state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthCheckStatus {
    /// Node is healthy and responding to requests
    Healthy,
    /// Node is unhealthy with the provided error message
    Unhealthy(String),
}

/// Circuit breaker state for each node.
///
/// The circuit breaker prevents cascading failures by skipping nodes that are
/// experiencing consecutive failures. State transitions:
///
/// - **Closed → Open**: When consecutive failures exceed threshold
/// - **Open → HalfOpen**: After exponential backoff timeout elapses
/// - **HalfOpen → Closed**: On successful health check
/// - **HalfOpen → Open**: On failed health check
///
/// Nodes in Open state are skipped by `LoadBalancer::next_node()` without
/// attempting connection, enabling fast fail behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerState {
    /// Normal operation, requests flow through
    Closed,
    /// Circuit is tripped, requests fail fast without reaching the node
    ///
    /// When in this state, the load balancer will skip this node entirely
    /// without attempting connections, preventing wasted time on likely failures.
    Open,
    /// Testing if the node has recovered
    ///
    /// In this state, the node is selectable by the load balancer. A successful
    /// health check will transition to Closed, while a failure will transition
    /// back to Open (with increased backoff).
    HalfOpen,
}

/// Circuit breaker configuration.
///
/// Controls when circuits trip and how long they wait before attempting recovery.
/// Uses exponential backoff to avoid overwhelming recovering nodes.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before tripping the circuit
    ///
    /// Default: 5
    pub failure_threshold: u32,
    /// Base timeout before attempting half-open (in seconds)
    ///
    /// Actual timeout is calculated as: `base_timeout_secs * backoff_multiplier^(failures - 1)`
    /// For example, with base_timeout_secs=30 and multiplier=2.0:
    /// - 1st trip: 30s
    /// - 2nd trip: 60s
    /// - 3rd trip: 120s
    /// - Capped at max_timeout_secs
    ///
    /// Default: 30 seconds
    pub base_timeout_secs: u64,
    /// Maximum timeout cap (in seconds)
    ///
    /// Default: 300 seconds (5 minutes)
    pub max_timeout_secs: u64,
    /// Exponential backoff multiplier
    ///
    /// Default: 2.0 (doubles timeout each time)
    pub backoff_multiplier: f64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            base_timeout_secs: 30,
            max_timeout_secs: 300, // 5 minutes
            backoff_multiplier: 2.0,
        }
    }
}

impl CircuitBreakerConfig {
    /// Calculates timeout with exponential backoff based on consecutive failures.
    ///
    /// This implements exponential backoff to avoid overwhelming nodes that are
    /// struggling to recover. The timeout increases with each consecutive failure
    /// but is capped at `max_timeout_secs`.
    ///
    /// # Formula
    /// ```text
    /// timeout = min(base_timeout_ms * multiplier^(failures - 1), max_timeout_ms)
    /// ```
    ///
    /// # Arguments
    /// * `consecutive_failures` - Number of consecutive failures (must be >= 1)
    ///
    /// # Returns
    /// The duration to wait before attempting half-open state
    ///
    /// # Example
    /// ```rust
    /// use madrpc_orchestrator::CircuitBreakerConfig;
    /// use std::time::Duration;
    ///
    /// let config = CircuitBreakerConfig::default();
    /// assert_eq!(config.calculate_timeout(1), Duration::from_secs(30));
    /// assert_eq!(config.calculate_timeout(2), Duration::from_secs(60));
    /// assert_eq!(config.calculate_timeout(3), Duration::from_secs(120));
    /// ```
    pub fn calculate_timeout(&self, consecutive_failures: u32) -> std::time::Duration {
        let base_ms = self.base_timeout_secs * 1000;
        let multiplier = self.backoff_multiplier.powi(consecutive_failures as i32 - 1);
        let backoff_ms = (base_ms as f64 * multiplier) as u64;
        let max_ms = self.max_timeout_secs * 1000;
        std::time::Duration::from_millis(backoff_ms.min(max_ms))
    }
}

/// A node in the load balancer with its current state.
///
/// Each node tracks its own health status, circuit breaker state, and failure history.
/// This state is updated by the health checker and used by the load balancer for
/// routing decisions.
#[derive(Debug, Clone)]
pub struct Node {
    /// Node address (e.g., "127.0.0.1:9001")
    pub addr: String,
    /// Whether the node is currently enabled
    ///
    /// Nodes can be disabled manually (via API) or automatically (by health checker).
    /// Disabled nodes are excluded from round-robin selection.
    pub enabled: bool,
    /// Reason why the node is disabled (if disabled)
    pub disable_reason: Option<DisableReason>,
    /// Number of consecutive health check failures
    ///
    /// Reset to 0 on successful health check. Used by circuit breaker to determine
    /// when to trip and to calculate exponential backoff.
    pub consecutive_failures: u32,
    /// Timestamp of the last health check
    pub last_health_check: Option<Instant>,
    /// Result of the last health check
    pub last_health_check_status: Option<HealthCheckStatus>,
    /// Circuit breaker state
    ///
    /// Nodes in Open state are skipped by the load balancer without attempting
    /// connection, enabling fast fail behavior.
    pub circuit_state: CircuitBreakerState,
    /// When the circuit was opened (for timeout calculation)
    ///
    /// Used to determine when the circuit should transition from Open to HalfOpen
    /// based on exponential backoff.
    pub circuit_opened_at: Option<SystemTime>,
    /// Total number of requests forwarded to this node
    pub request_count: u64,
    /// Timestamp of the last request to this node
    pub last_request_time: Option<SystemTime>,
}

impl Node {
    /// Creates a new node with default state.
    ///
    /// New nodes start in enabled state with closed circuit breaker and no failures.
    ///
    /// # Arguments
    /// * `addr` - The node address (e.g., "127.0.0.1:9001")
    ///
    /// # Returns
    /// A new Node instance with:
    /// - enabled: true
    /// - circuit_state: Closed
    /// - consecutive_failures: 0
    pub fn new(addr: String) -> Self {
        Self {
            addr,
            enabled: true,
            disable_reason: None,
            consecutive_failures: 0,
            last_health_check: None,
            last_health_check_status: None,
            circuit_state: CircuitBreakerState::Closed,
            circuit_opened_at: None,
            request_count: 0,
            last_request_time: None,
        }
    }

    /// Checks if the circuit should transition to half-open based on timeout.
    ///
    /// This method is called by the health checker to determine if enough time
    /// has passed for a node in Open state to attempt recovery.
    ///
    /// # Arguments
    /// * `config` - The circuit breaker configuration
    ///
    /// # Returns
    /// `true` if the circuit has been open long enough to attempt half-open
    ///
    /// # Behavior
    /// - Returns `false` if circuit is not Open (already Closed or HalfOpen)
    /// - Returns `false` if no `circuit_opened_at` timestamp is set
    /// - Returns `true` if elapsed time >= calculated exponential backoff timeout
    pub fn should_attempt_half_open(&self, config: &CircuitBreakerConfig) -> bool {
        if self.circuit_state != CircuitBreakerState::Open {
            return false;
        }

        if let Some(opened_at) = self.circuit_opened_at {
            let elapsed = opened_at
                .elapsed()
                .unwrap_or(std::time::Duration::from_secs(0));
            let timeout = config.calculate_timeout(self.consecutive_failures);
            elapsed >= timeout
        } else {
            false
        }
    }

    /// Transitions circuit breaker state and updates timestamp.
    ///
    /// This method handles the side effects of state transitions:
    /// - **Open**: Sets `circuit_opened_at` to now (for timeout calculation)
    /// - **Closed**: Clears `circuit_opened_at`
    /// - **HalfOpen**: Clears `circuit_opened_at`
    ///
    /// # Arguments
    /// * `new_state` - The new circuit breaker state
    pub fn transition_circuit_state(&mut self, new_state: CircuitBreakerState) {
        self.circuit_state = new_state;
        match new_state {
            CircuitBreakerState::Open => {
                self.circuit_opened_at = Some(SystemTime::now());
            }
            CircuitBreakerState::Closed | CircuitBreakerState::HalfOpen => {
                self.circuit_opened_at = None;
            }
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
        assert_eq!(node.circuit_state, CircuitBreakerState::Closed);
        assert!(node.circuit_opened_at.is_none());
        assert_eq!(node.request_count, 0);
        assert!(node.last_request_time.is_none());
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

    #[test]
    fn test_circuit_breaker_state_equality() {
        assert_eq!(CircuitBreakerState::Closed, CircuitBreakerState::Closed);
        assert_eq!(CircuitBreakerState::Open, CircuitBreakerState::Open);
        assert_eq!(CircuitBreakerState::HalfOpen, CircuitBreakerState::HalfOpen);
        assert_ne!(CircuitBreakerState::Closed, CircuitBreakerState::Open);
        assert_ne!(CircuitBreakerState::Open, CircuitBreakerState::HalfOpen);
    }

    #[test]
    fn test_circuit_breaker_config_default() {
        let config = CircuitBreakerConfig::default();
        assert_eq!(config.failure_threshold, 5);
        assert_eq!(config.base_timeout_secs, 30);
        assert_eq!(config.max_timeout_secs, 300);
        assert_eq!(config.backoff_multiplier, 2.0);
    }

    #[test]
    fn test_circuit_breaker_calculate_timeout() {
        let config = CircuitBreakerConfig::default();

        // First failure: 30 seconds
        let timeout1 = config.calculate_timeout(1);
        assert_eq!(timeout1.as_secs(), 30);

        // Second failure: 60 seconds (30 * 2)
        let timeout2 = config.calculate_timeout(2);
        assert_eq!(timeout2.as_secs(), 60);

        // Third failure: 120 seconds (30 * 2^2)
        let timeout3 = config.calculate_timeout(3);
        assert_eq!(timeout3.as_secs(), 120);

        // Fourth failure: 240 seconds (30 * 2^3)
        let timeout4 = config.calculate_timeout(4);
        assert_eq!(timeout4.as_secs(), 240);

        // Fifth failure: 300 seconds (capped at max)
        let timeout5 = config.calculate_timeout(5);
        assert_eq!(timeout5.as_secs(), 300);

        // Large failure count should cap at max
        let timeout10 = config.calculate_timeout(10);
        assert_eq!(timeout10.as_secs(), 300);
    }

    #[test]
    fn test_node_transition_to_open_sets_timestamp() {
        let mut node = Node::new("node1".to_string());
        assert_eq!(node.circuit_state, CircuitBreakerState::Closed);
        assert!(node.circuit_opened_at.is_none());

        node.transition_circuit_state(CircuitBreakerState::Open);
        assert_eq!(node.circuit_state, CircuitBreakerState::Open);
        assert!(node.circuit_opened_at.is_some());
    }

    #[test]
    fn test_node_transition_to_closed_clears_timestamp() {
        let mut node = Node::new("node1".to_string());
        node.transition_circuit_state(CircuitBreakerState::Open);
        assert!(node.circuit_opened_at.is_some());

        node.transition_circuit_state(CircuitBreakerState::Closed);
        assert_eq!(node.circuit_state, CircuitBreakerState::Closed);
        assert!(node.circuit_opened_at.is_none());
    }

    #[test]
    fn test_node_transition_to_half_open_clears_timestamp() {
        let mut node = Node::new("node1".to_string());
        node.transition_circuit_state(CircuitBreakerState::Open);
        assert!(node.circuit_opened_at.is_some());

        node.transition_circuit_state(CircuitBreakerState::HalfOpen);
        assert_eq!(node.circuit_state, CircuitBreakerState::HalfOpen);
        assert!(node.circuit_opened_at.is_none());
    }

    #[test]
    fn test_should_attempt_half_open_when_closed() {
        let node = Node::new("node1".to_string());
        let config = CircuitBreakerConfig::default();
        assert!(!node.should_attempt_half_open(&config));
    }

    #[test]
    fn test_should_attempt_half_open_when_half_open() {
        let mut node = Node::new("node1".to_string());
        node.transition_circuit_state(CircuitBreakerState::HalfOpen);
        let config = CircuitBreakerConfig::default();
        assert!(!node.should_attempt_half_open(&config));
    }

    #[test]
    fn test_should_attempt_half_open_before_timeout() {
        let mut node = Node::new("node1".to_string());
        node.consecutive_failures = 3;
        node.transition_circuit_state(CircuitBreakerState::Open);
        let config = CircuitBreakerConfig::default();
        // Just opened, should not attempt half-open yet
        assert!(!node.should_attempt_half_open(&config));
    }

    #[test]
    fn test_should_attempt_half_open_after_timeout() {
        let mut node = Node::new("node1".to_string());
        node.consecutive_failures = 1;
        node.transition_circuit_state(CircuitBreakerState::Open);

        // Manually set opened_at to 31 seconds ago
        let past = SystemTime::now() - std::time::Duration::from_secs(31);
        node.circuit_opened_at = Some(past);

        let config = CircuitBreakerConfig::default();
        // Should attempt half-open after timeout
        assert!(node.should_attempt_half_open(&config));
    }
}
