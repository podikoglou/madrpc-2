use crate::node::{CircuitBreakerConfig, CircuitBreakerState, DisableReason, HealthCheckStatus, Node};
use std::collections::HashMap;

/// Round-robin load balancer for nodes with circuit breaker.
///
/// The load balancer distributes incoming requests across compute nodes using
/// round-robin selection while respecting node state (enabled/disabled) and
/// circuit breaker status.
///
/// # Data Structures
///
/// Uses two complementary data structures for optimal performance:
/// - **HashMap**: O(1) lookups by address for state updates
/// - **Vec**: Maintains order for round-robin iteration over enabled nodes
///
/// # Circuit Breaker Integration
///
/// Nodes with Open circuits are automatically skipped during round-robin selection
/// without attempting connection, enabling fast fail behavior. This prevents
/// wasting time on nodes that are likely to fail.
///
/// # Thread Safety
///
/// This struct is not thread-safe by itself. It should be wrapped in
/// `Arc<RwLock<LoadBalancer>>` for concurrent access (as done by `Orchestrator`).
pub struct LoadBalancer {
    /// All nodes indexed by address for O(1) lookups
    nodes: HashMap<String, Node>,
    /// Enabled node addresses for O(1) round-robin iteration
    enabled_nodes: Vec<String>,
    /// Current index in round-robin iteration
    ///
    /// Wraps around using modulo arithmetic to prevent overflow.
    round_robin_index: usize,
    /// Circuit breaker configuration
    circuit_config: CircuitBreakerConfig,
}

impl LoadBalancer {
    /// Creates a new load balancer with a static node list and default circuit breaker config.
    ///
    /// All nodes start enabled with closed circuit breakers. The round-robin index
    /// starts at 0 and wraps around using modulo arithmetic.
    ///
    /// # Arguments
    /// * `node_addrs` - List of node addresses (e.g., "127.0.0.1:9001")
    ///
    /// # Returns
    /// A new LoadBalancer instance
    ///
    /// # Example
    /// ```rust
    /// use madrpc_orchestrator::LoadBalancer;
    ///
    /// let lb = LoadBalancer::new(vec![
    ///     "127.0.0.1:9001".to_string(),
    ///     "127.0.0.1:9002".to_string(),
    /// ]);
    /// ```
    pub fn new(node_addrs: Vec<String>) -> Self {
        let enabled_nodes = node_addrs.clone();
        let nodes = node_addrs.into_iter().map(|addr| (addr.clone(), Node::new(addr))).collect();
        Self {
            nodes,
            enabled_nodes,
            round_robin_index: 0,
            circuit_config: CircuitBreakerConfig::default(),
        }
    }

    /// Creates a new load balancer with custom circuit breaker config.
    ///
    /// Use this when you need non-default circuit breaker behavior (e.g., different
    /// failure thresholds or backoff multipliers).
    ///
    /// # Arguments
    /// * `node_addrs` - List of node addresses
    /// * `circuit_config` - Circuit breaker configuration
    ///
    /// # Returns
    /// A new LoadBalancer instance with custom circuit breaker config
    pub fn with_config(node_addrs: Vec<String>, circuit_config: CircuitBreakerConfig) -> Self {
        let enabled_nodes = node_addrs.clone();
        let nodes = node_addrs.into_iter().map(|addr| (addr.clone(), Node::new(addr))).collect();
        Self {
            nodes,
            enabled_nodes,
            round_robin_index: 0,
            circuit_config,
        }
    }

    /// Gets the next ENABLED node using round-robin, skipping nodes with open circuits.
    ///
    /// This is the main method used by the orchestrator to select a node for each
    /// incoming request. It implements both round-robin load balancing and circuit
    /// breaker fast-fail behavior.
    ///
    /// # Algorithm
    /// 1. Return None if no enabled nodes exist
    /// 2. Iterate through enabled nodes starting at current index
    /// 3. Skip nodes with Open circuits (fail fast)
    /// 4. Return first node with Closed or HalfOpen circuit
    /// 5. Return None if all nodes have Open circuits
    ///
    /// # Returns
    /// - `Some(addr)` - Address of next available node
    /// - `None` - No enabled nodes or all circuits are open
    ///
    /// # Note
    /// The round-robin index always advances, even when skipping nodes with open circuits.
    /// This ensures even distribution when nodes recover.
    pub fn next_node(&mut self) -> Option<String> {
        if self.enabled_nodes.is_empty() {
            return None;
        }

        // Find a node that's not in Open state (circuit breaker)
        for _ in 0..self.enabled_nodes.len() {
            let idx = self.round_robin_index % self.enabled_nodes.len();
            self.round_robin_index = self.round_robin_index.wrapping_add(1) % self.enabled_nodes.len();

            if let Some(node) = self.nodes.get(&self.enabled_nodes[idx]) {
                // Skip nodes with open circuits (fail fast)
                if node.circuit_state != CircuitBreakerState::Open {
                    return Some(self.enabled_nodes[idx].clone());
                }
            }
        }

        // All nodes have open circuits, return None
        None
    }

    /// Manually disables a node (indefinite).
    ///
    /// Manually disabled nodes are marked with `DisableReason::Manual` and will
    /// never be auto-re-enabled by the health checker. They must be manually
    /// re-enabled via `enable_node()`.
    ///
    /// # Arguments
    /// * `addr` - The node address to disable
    ///
    /// # Returns
    /// - `true` - Node was found and disabled
    /// - `false` - Node was not found
    pub fn disable_node(&mut self, addr: &str) -> bool {
        // O(1) HashMap lookup
        if let Some(node) = self.nodes.get_mut(addr) {
            node.enabled = false;
            node.disable_reason = Some(DisableReason::Manual);
            // O(N) removal from enabled_nodes vec, but this is infrequent
            self.enabled_nodes.retain(|a| a != addr);
            true
        } else {
            false
        }
    }

    /// Manually enables a node.
    ///
    /// This resets the node's disable reason, consecutive failures, and adds it
    /// back to the enabled nodes list if not already present.
    ///
    /// # Arguments
    /// * `addr` - The node address to enable
    ///
    /// # Returns
    /// - `true` - Node was found and enabled
    /// - `false` - Node was not found
    pub fn enable_node(&mut self, addr: &str) -> bool {
        // O(1) HashMap lookup
        if let Some(node) = self.nodes.get_mut(addr) {
            node.enabled = true;
            node.disable_reason = None;
            node.consecutive_failures = 0;
            // Add to enabled_nodes if not already present
            if !self.enabled_nodes.contains(&addr.to_string()) {
                self.enabled_nodes.push(addr.to_string());
            }
            true
        } else {
            false
        }
    }

    /// Auto-disables a node due to health check failure.
    ///
    /// This method is called by the health checker when consecutive failures
    /// exceed the threshold. It only disables nodes that are not already manually
    /// disabled, preserving user intent.
    ///
    /// # Arguments
    /// * `addr` - The node address to disable
    ///
    /// # Returns
    /// - `true` - Node was auto-disabled
    /// - `false` - Node was manually disabled or not found
    ///
    /// # Note
    /// Auto-disabled nodes can be re-enabled by the health checker on recovery
    /// (via `auto_enable_node`), while manually disabled nodes cannot.
    pub fn auto_disable_node(&mut self, addr: &str) -> bool {
        // O(1) HashMap lookup
        if let Some(node) = self.nodes.get_mut(addr) {
            // Only auto-disable if not manually disabled
            if node.disable_reason != Some(DisableReason::Manual) {
                node.enabled = false;
                node.disable_reason = Some(DisableReason::HealthCheck);
                // Remove from enabled_nodes
                self.enabled_nodes.retain(|a| a != addr);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Auto-enables a node that recovered (only if it was auto-disabled).
    ///
    /// This method is called by the health checker when a previously unhealthy
    /// node passes a health check. It only works on nodes that were auto-disabled,
    /// preserving manual disable intent.
    ///
    /// # Arguments
    /// * `addr` - The node address to enable
    ///
    /// # Returns
    /// - `true` - Node was auto-enabled
    /// - `false` - Node was manually disabled or not found
    pub fn auto_enable_node(&mut self, addr: &str) -> bool {
        // O(1) HashMap lookup
        if let Some(node) = self.nodes.get_mut(addr) {
            if node.disable_reason == Some(DisableReason::HealthCheck) {
                node.enabled = true;
                node.disable_reason = None;
                node.consecutive_failures = 0;
                // Add to enabled_nodes if not already present
                if !self.enabled_nodes.contains(&addr.to_string()) {
                    self.enabled_nodes.push(addr.to_string());
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Updates health check status for a node with circuit breaker logic.
    ///
    /// This is the main integration point between health checking and circuit breaking.
    /// It handles state transitions based on health check results:
    ///
    /// **Healthy**:
    /// - Resets consecutive failures to 0
    /// - Transitions HalfOpen → Closed (circuit recovered)
    ///
    /// **Unhealthy**:
    /// - Increments consecutive failures
    /// - Closed → Open (trip circuit if threshold reached)
    /// - HalfOpen → Open (failed recovery attempt)
    /// - Open → Open (already tripped, no change)
    ///
    /// # Arguments
    /// * `addr` - The node address
    /// * `status` - The health check status
    pub fn update_health_status(&mut self, addr: &str, status: HealthCheckStatus) {
        // O(1) HashMap lookup
        if let Some(node) = self.nodes.get_mut(addr) {
            node.last_health_check = Some(std::time::Instant::now());
            node.last_health_check_status = Some(status.clone());

            match status {
                HealthCheckStatus::Healthy => {
                    // Reset failures on success
                    node.consecutive_failures = 0;

                    // If in half-open, transition to closed
                    if node.circuit_state == CircuitBreakerState::HalfOpen {
                        node.transition_circuit_state(CircuitBreakerState::Closed);
                    }
                }
                HealthCheckStatus::Unhealthy(_) => {
                    node.consecutive_failures += 1;

                    // Circuit breaker state transitions
                    match node.circuit_state {
                        CircuitBreakerState::Closed => {
                            // Trip the circuit if threshold reached
                            if node.consecutive_failures >= self.circuit_config.failure_threshold {
                                node.transition_circuit_state(CircuitBreakerState::Open);
                            }
                        }
                        CircuitBreakerState::HalfOpen => {
                            // Failed in half-open, trip back to open
                            node.transition_circuit_state(CircuitBreakerState::Open);
                        }
                        CircuitBreakerState::Open => {
                            // Already open, nothing to do
                        }
                    }
                }
            }
        }
    }

    /// Checks and updates circuit breaker state for timeout transitions.
    ///
    /// This method is called by the health checker before each round of health
    /// checks to determine if any nodes in Open state should transition to
    /// HalfOpen based on exponential backoff timeout.
    ///
    /// # Returns
    /// - `true` - At least one node transitioned from Open to HalfOpen
    /// - `false` - No transitions occurred
    pub fn check_circuit_timeouts(&mut self) -> bool {
        let mut any_transitioned = false;

        for node in self.nodes.values_mut() {
            if node.should_attempt_half_open(&self.circuit_config) {
                node.transition_circuit_state(CircuitBreakerState::HalfOpen);
                any_transitioned = true;
            }
        }

        any_transitioned
    }

    /// Gets circuit breaker state for a node.
    ///
    /// # Arguments
    /// * `addr` - The node address
    ///
    /// # Returns
    /// - `Some(state)` - Circuit breaker state if node exists
    /// - `None` - Node not found
    pub fn circuit_state(&self, addr: &str) -> Option<CircuitBreakerState> {
        self.nodes.get(addr).map(|n| n.circuit_state)
    }

    /// Gets all nodes with open circuits.
    ///
    /// This is useful for monitoring and debugging to see which nodes are
    /// currently being skipped by the load balancer.
    ///
    /// # Returns
    /// Vector of node addresses with Open circuit state
    pub fn open_circuit_nodes(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.circuit_state == CircuitBreakerState::Open)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Gets all nodes with half-open circuits.
    ///
    /// Nodes in HalfOpen state are testing recovery and will transition to
    /// Closed on success or Open on failure.
    ///
    /// # Returns
    /// Vector of node addresses with HalfOpen circuit state
    pub fn half_open_circuit_nodes(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.circuit_state == CircuitBreakerState::HalfOpen)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Gets consecutive failures for a node.
    ///
    /// # Arguments
    /// * `addr` - The node address
    ///
    /// # Returns
    /// Number of consecutive health check failures (0 if node not found)
    pub fn consecutive_failures(&self, addr: &str) -> u32 {
        // O(1) HashMap lookup
        self.nodes
            .get(addr)
            .map(|n| n.consecutive_failures)
            .unwrap_or(0)
    }

    /// Gets all nodes (including disabled ones).
    ///
    /// This returns cloned Node instances, which include all state information
    /// (health status, circuit state, etc.).
    ///
    /// # Returns
    /// Vector of all nodes with their current state
    pub fn all_nodes(&self) -> Vec<Node> {
        self.nodes.values().cloned().collect()
    }

    /// Gets enabled nodes only.
    ///
    /// This is an O(1) operation since it clones the pre-populated enabled_nodes vec.
    ///
    /// # Returns
    /// Vector of enabled node addresses
    pub fn enabled_nodes(&self) -> Vec<String> {
        // O(1) - just clone the enabled_nodes vec
        self.enabled_nodes.clone()
    }

    /// Gets disabled nodes only.
    ///
    /// This is an O(N) operation used for display purposes (e.g., monitoring UI).
    ///
    /// # Returns
    /// Vector of disabled node addresses
    pub fn disabled_nodes(&self) -> Vec<String> {
        // O(N) but this is only used for display purposes
        self.nodes
            .iter()
            .filter(|(_, n)| !n.enabled)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Adds a node to the pool.
    ///
    /// New nodes are enabled by default with closed circuit breakers. Duplicate
    /// nodes are ignored (no-op).
    ///
    /// # Arguments
    /// * `node_addr` - The node address to add
    pub fn add_node(&mut self, node_addr: String) {
        // O(1) HashMap contains check
        if !self.nodes.contains_key(&node_addr) {
            self.nodes.insert(node_addr.clone(), Node::new(node_addr.clone()));
            // New nodes are enabled by default
            self.enabled_nodes.push(node_addr);
        }
    }

    /// Removes a node from the pool.
    ///
    /// This removes the node from both the HashMap and the enabled_nodes vec.
    /// If the node is already disabled, this is a no-op for the enabled_nodes vec.
    ///
    /// # Arguments
    /// * `node_addr` - The node address to remove
    pub fn remove_node(&mut self, node_addr: &str) {
        // O(1) HashMap remove
        self.nodes.remove(node_addr);
        // Also remove from enabled_nodes
        self.enabled_nodes.retain(|a| a != node_addr);
    }

    /// Gets the number of nodes (including disabled ones).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Gets the number of enabled nodes.
    ///
    /// This is an O(1) operation.
    pub fn enabled_count(&self) -> usize {
        // O(1) - just return the length of enabled_nodes
        self.enabled_nodes.len()
    }

    /// Gets list of all node addresses (backward compatibility).
    ///
    /// Returns addresses of all nodes, including disabled ones. This is an O(N)
    /// operation used primarily for display purposes.
    ///
    /// # Returns
    /// Vector of all node addresses
    pub fn nodes(&self) -> Vec<String> {
        // O(N) but this is only used for display
        self.nodes.keys().cloned().collect()
    }

    /// Test helper to get mutable access to a node's circuit state.
    ///
    /// This allows tests to manipulate node state directly for testing circuit
    /// breaker behavior without exposing internal mutability in the public API.
    #[cfg(test)]
    pub fn with_node_state<F>(&mut self, addr: &str, f: F)
    where
        F: FnOnce(&mut Node),
    {
        if let Some(node) = self.nodes.get_mut(addr) {
            f(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_balancer_creation() {
        let nodes = vec!["localhost:9001".to_string(), "localhost:9002".to_string()];
        let lb = LoadBalancer::new(nodes.clone());
        assert_eq!(lb.node_count(), 2);
    }

    #[test]
    fn test_round_robin() {
        let nodes = vec!["node1".to_string(), "node2".to_string(), "node3".to_string()];
        let mut lb = LoadBalancer::new(nodes);

        assert_eq!(lb.next_node(), Some("node1".to_string()));
        assert_eq!(lb.next_node(), Some("node2".to_string()));
        assert_eq!(lb.next_node(), Some("node3".to_string()));
        assert_eq!(lb.next_node(), Some("node1".to_string()));
        // wraps around
    }

    #[test]
    fn test_empty_nodes_returns_none() {
        let mut lb = LoadBalancer::new(vec![]);
        assert_eq!(lb.next_node(), None);
    }

    #[test]
    fn test_single_node() {
        let mut lb = LoadBalancer::new(vec!["only-node".to_string()]);
        assert_eq!(lb.next_node(), Some("only-node".to_string()));
        assert_eq!(lb.next_node(), Some("only-node".to_string()));
    }

    #[test]
    fn test_add_node() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.add_node("node2".to_string());
        assert_eq!(lb.node_count(), 2);
    }

    #[test]
    fn test_add_duplicate_node() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.add_node("node1".to_string());
        // duplicate should not be added
        assert_eq!(lb.node_count(), 1);
    }

    #[test]
    fn test_remove_node() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);
        lb.remove_node("node2");
        assert_eq!(lb.node_count(), 2);
        let mut nodes = lb.nodes();
        nodes.sort(); // HashMap doesn't guarantee order
        assert_eq!(nodes, vec!["node1".to_string(), "node3".to_string()]);
    }

    #[test]
    fn test_get_nodes() {
        let nodes = vec!["a".to_string(), "b".to_string()];
        let lb = LoadBalancer::new(nodes.clone());
        let mut result = lb.nodes();
        result.sort(); // HashMap doesn't guarantee order
        assert_eq!(result, nodes);
    }

    #[test]
    fn test_manual_disable_node() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string(), "node2".to_string()]);
        assert!(lb.disable_node("node1"));

        // node1 should be disabled
        assert_eq!(lb.disabled_nodes(), vec!["node1".to_string()]);
        assert_eq!(lb.enabled_nodes(), vec!["node2".to_string()]);

        // round-robin should skip disabled node1
        assert_eq!(lb.next_node(), Some("node2".to_string()));
        assert_eq!(lb.next_node(), Some("node2".to_string()));
    }

    #[test]
    fn test_manual_enable_node() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.disable_node("node1");
        assert!(lb.enable_node("node1"));
        assert_eq!(lb.enabled_nodes(), vec!["node1".to_string()]);
    }

    #[test]
    fn test_auto_disable_node() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        assert!(lb.auto_disable_node("node1"));

        // Should have HealthCheck reason
        let nodes = lb.all_nodes();
        let node1 = nodes.iter().find(|n| n.addr == "node1").unwrap();
        assert!(!node1.enabled);
        assert_eq!(node1.disable_reason, Some(DisableReason::HealthCheck));
    }

    #[test]
    fn test_auto_disable_does_not_affect_manual() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.disable_node("node1"); // Manual disable

        // Auto-disable should not work on manually disabled node
        assert!(!lb.auto_disable_node("node1"));

        let nodes = lb.all_nodes();
        let node1 = nodes.iter().find(|n| n.addr == "node1").unwrap();
        assert_eq!(node1.disable_reason, Some(DisableReason::Manual));
    }

    #[test]
    fn test_auto_enable_node() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.auto_disable_node("node1");
        assert!(lb.auto_enable_node("node1"));

        assert_eq!(lb.enabled_nodes(), vec!["node1".to_string()]);
    }

    #[test]
    fn test_auto_enable_does_not_affect_manual() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.disable_node("node1"); // Manual disable

        // Auto-enable should not work on manually disabled node
        assert!(!lb.auto_enable_node("node1"));
        assert_eq!(lb.enabled_nodes(), vec![] as Vec<String>);
    }

    #[test]
    fn test_update_health_status_healthy() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.update_health_status("node1", HealthCheckStatus::Healthy);

        let nodes = lb.all_nodes();
        let node1 = nodes.iter().find(|n| n.addr == "node1").unwrap();
        assert_eq!(node1.consecutive_failures, 0);
        assert_eq!(
            node1.last_health_check_status,
            Some(HealthCheckStatus::Healthy)
        );
        assert!(node1.last_health_check.is_some());
    }

    #[test]
    fn test_update_health_status_unhealthy() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("error".to_string()));

        let nodes = lb.all_nodes();
        let node1 = nodes.iter().find(|n| n.addr == "node1").unwrap();
        assert_eq!(node1.consecutive_failures, 1);
        assert_eq!(
            node1.last_health_check_status,
            Some(HealthCheckStatus::Unhealthy("error".to_string()))
        );
    }

    #[test]
    fn test_consecutive_failures() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        assert_eq!(lb.consecutive_failures("node1"), 0);

        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(lb.consecutive_failures("node1"), 1);

        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(lb.consecutive_failures("node1"), 2);

        lb.update_health_status("node1", HealthCheckStatus::Healthy);
        assert_eq!(lb.consecutive_failures("node1"), 0);
    }

    #[test]
    fn test_all_nodes_disabled_returns_none() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string(), "node2".to_string()]);
        lb.disable_node("node1");
        lb.disable_node("node2");
        assert_eq!(lb.next_node(), None);
    }

    #[test]
    fn test_enabled_count() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);
        assert_eq!(lb.enabled_count(), 3);

        lb.disable_node("node1");
        assert_eq!(lb.enabled_count(), 2);

        lb.disable_node("node2");
        assert_eq!(lb.enabled_count(), 1);
    }

    #[test]
    fn test_round_robin_index_wraps_at_usize_max() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);
        lb.round_robin_index = usize::MAX;
        assert_eq!(lb.next_node(), Some("node1".to_string()));
        assert_eq!(lb.round_robin_index, 0);
    }

    #[test]
    fn test_round_robin_with_disabled_nodes() {
        let nodes = vec!["node1".to_string(), "node2".to_string(), "node3".to_string()];
        let mut lb = LoadBalancer::new(nodes);
        lb.disable_node("node2");
        assert_eq!(lb.next_node(), Some("node1".to_string()));
        assert_eq!(lb.next_node(), Some("node3".to_string()));
        assert_eq!(lb.next_node(), Some("node1".to_string()));
        assert_eq!(lb.next_node(), Some("node3".to_string()));
    }

    #[test]
    fn test_concurrent_round_robin_calls() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let lb = Arc::new(Mutex::new(LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
            "node4".to_string(),
        ])));

        let mut handles = vec![];
        for _ in 0..10 {
            let lb_clone = Arc::clone(&lb);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    let mut lb = lb_clone.lock().unwrap();
                    let result = lb.next_node();
                    assert!(result.is_some());
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_round_robin_distributes_evenly() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);

        let mut counts = std::collections::HashMap::new();
        for _ in 0..300 {
            let node = lb.next_node().unwrap();
            *counts.entry(node).or_insert(0) += 1;
        }

        assert_eq!(counts["node1"], 100);
        assert_eq!(counts["node2"], 100);
        assert_eq!(counts["node3"], 100);
    }

    // ============================================================================
    // Circuit Breaker Tests
    // ============================================================================

    #[test]
    fn test_circuit_breaker_initially_closed() {
        let lb = LoadBalancer::new(vec!["node1".to_string()]);
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Closed)
        );
    }

    #[test]
    fn test_circuit_breaker_trips_after_threshold() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);

        // Default threshold is 5
        for _ in 0..4 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
            assert_eq!(
                lb.circuit_state("node1"),
                Some(CircuitBreakerState::Closed)
            );
        }

        // 5th failure should trip the circuit
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );
    }

    #[test]
    fn test_circuit_breaker_custom_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            ..Default::default()
        };
        let mut lb = LoadBalancer::with_config(vec!["node1".to_string()], config);

        // First failure
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Closed)
        );

        // Second failure should trip
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );
    }

    #[test]
    fn test_circuit_breaker_opens_circuit_skipped() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);

        // Trip node1's circuit
        for _ in 0..5 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        }

        // next_node should skip node1 (open circuit)
        assert_eq!(lb.next_node(), Some("node2".to_string()));
        assert_eq!(lb.next_node(), Some("node3".to_string()));
        assert_eq!(lb.next_node(), Some("node2".to_string()));
        assert_eq!(lb.next_node(), Some("node3".to_string()));
    }

    #[test]
    fn test_circuit_breaker_all_open_returns_none() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);

        // Trip all circuits
        for addr in &["node1", "node2", "node3"] {
            for _ in 0..5 {
                lb.update_health_status(addr, HealthCheckStatus::Unhealthy("err".to_string()));
            }
        }

        // All circuits open, should return None
        assert_eq!(lb.next_node(), None);
    }

    #[test]
    fn test_circuit_breaker_reset_on_success() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);

        // Trip the circuit
        for _ in 0..5 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        }
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );

        // Success should reset failures but NOT close circuit (must wait for timeout)
        lb.update_health_status("node1", HealthCheckStatus::Healthy);
        assert_eq!(lb.consecutive_failures("node1"), 0);
        // Circuit still open because it was opened before success
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );
    }

    #[test]
    fn test_circuit_breaker_half_open_to_closed_on_success() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);

        // Trip the circuit
        for _ in 0..5 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        }

        // Manually transition to half-open (simulating timeout)
        lb.with_node_state("node1", |node| {
            node.transition_circuit_state(CircuitBreakerState::HalfOpen);
        });

        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::HalfOpen)
        );

        // Success should close the circuit
        lb.update_health_status("node1", HealthCheckStatus::Healthy);
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Closed)
        );
    }

    #[test]
    fn test_circuit_breaker_half_open_to_open_on_failure() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);

        // Trip the circuit
        for _ in 0..5 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        }

        // Manually transition to half-open
        lb.with_node_state("node1", |node| {
            node.transition_circuit_state(CircuitBreakerState::HalfOpen);
        });

        // Failure in half-open should trip back to open
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );
    }

    #[test]
    fn test_get_open_circuit_nodes() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);

        // Trip node1 and node2
        for addr in &["node1", "node2"] {
            for _ in 0..5 {
                lb.update_health_status(addr, HealthCheckStatus::Unhealthy("err".to_string()));
            }
        }

        let open_nodes = lb.open_circuit_nodes();
        assert_eq!(open_nodes.len(), 2);
        assert!(open_nodes.contains(&"node1".to_string()));
        assert!(open_nodes.contains(&"node2".to_string()));
        assert!(!open_nodes.contains(&"node3".to_string()));
    }

    #[test]
    fn test_get_half_open_circuit_nodes() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);

        // Trip node1 and node2, then set node2 to half-open
        for addr in &["node1", "node2"] {
            for _ in 0..5 {
                lb.update_health_status(addr, HealthCheckStatus::Unhealthy("err".to_string()));
            }
        }

        // Manually set node2 to half-open
        lb.with_node_state("node2", |node| {
            node.transition_circuit_state(CircuitBreakerState::HalfOpen);
        });

        let half_open_nodes = lb.half_open_circuit_nodes();
        assert_eq!(half_open_nodes.len(), 1);
        assert_eq!(half_open_nodes[0], "node2".to_string());
    }

    #[test]
    fn test_check_circuit_timeouts_transitions_to_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            base_timeout_secs: 1,
            max_timeout_secs: 10,
            backoff_multiplier: 2.0,
        };
        let mut lb = LoadBalancer::with_config(vec!["node1".to_string()], config);

        // Trip the circuit with 2 failures
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));

        // Set opened_at to 2 seconds ago (past the 1s base timeout for 2 failures: 1 * 2^1 = 2s)
        lb.with_node_state("node1", |node| {
            node.circuit_opened_at = Some(std::time::SystemTime::now() - std::time::Duration::from_secs(3));
        });

        // Check timeouts should transition to half-open
        let transitioned = lb.check_circuit_timeouts();
        assert!(transitioned);
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::HalfOpen)
        );
    }

    #[test]
    fn test_check_circuit_timeops_no_transition_before_timeout() {
        let mut lb = LoadBalancer::new(vec!["node1".to_string()]);

        // Trip the circuit
        for _ in 0..5 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        }

        // Just opened, no timeout yet
        let transitioned = lb.check_circuit_timeouts();
        assert!(!transitioned);
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );
    }

    #[test]
    fn test_circuit_breaker_exponential_backoff_calculation() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            base_timeout_secs: 10,
            max_timeout_secs: 60,
            backoff_multiplier: 2.0,
        };

        // Check different timeout calculations
        assert_eq!(config.calculate_timeout(1).as_secs(), 10);
        assert_eq!(config.calculate_timeout(2).as_secs(), 20);
        assert_eq!(config.calculate_timeout(3).as_secs(), 40);
        assert_eq!(config.calculate_timeout(4).as_secs(), 60); // capped
        assert_eq!(config.calculate_timeout(10).as_secs(), 60); // capped
    }

    #[test]
    fn test_next_node_skips_open_but_allows_half_open() {
        let mut lb = LoadBalancer::new(vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ]);

        // Trip node1 circuit
        for _ in 0..5 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        }

        // Set node2 to half-open (should still be selectable)
        lb.with_node_state("node2", |node| {
            node.transition_circuit_state(CircuitBreakerState::HalfOpen);
        });

        // Should skip node1 (open) but include node2 (half-open) and node3 (closed)
        let nodes: Vec<_> = (0..10).map(|_| lb.next_node().unwrap()).collect();
        assert!(!nodes.contains(&"node1".to_string()));
        assert!(nodes.contains(&"node2".to_string()));
        assert!(nodes.contains(&"node3".to_string()));
    }

    #[test]
    fn test_circuit_breaker_with_custom_config() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            base_timeout_secs: 10,
            max_timeout_secs: 60,
            backoff_multiplier: 2.0,
        };

        let mut lb = LoadBalancer::with_config(vec!["node1".to_string()], config.clone());

        // Should trip after 3 failures
        for _ in 0..2 {
            lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
            assert_eq!(
                lb.circuit_state("node1"),
                Some(CircuitBreakerState::Closed)
            );
        }

        lb.update_health_status("node1", HealthCheckStatus::Unhealthy("err".to_string()));
        assert_eq!(
            lb.circuit_state("node1"),
            Some(CircuitBreakerState::Open)
        );
    }
}
