use crate::node::{DisableReason, HealthCheckStatus, Node};
use std::collections::HashMap;

/// Round-robin load balancer for nodes
pub struct LoadBalancer {
    /// All nodes indexed by address for O(1) lookups
    nodes: HashMap<String, Node>,
    /// Enabled node addresses for O(1) round-robin iteration
    enabled_nodes: Vec<String>,
    /// Current index in round-robin iteration
    round_robin_index: usize,
}

impl LoadBalancer {
    /// Create a new load balancer with a static node list
    pub fn new(node_addrs: Vec<String>) -> Self {
        let enabled_nodes = node_addrs.clone();
        let nodes = node_addrs.into_iter().map(|addr| (addr.clone(), Node::new(addr))).collect();
        Self {
            nodes,
            enabled_nodes,
            round_robin_index: 0,
        }
    }

    /// Get the next ENABLED node using round-robin
    pub fn next_node(&mut self) -> Option<String> {
        if self.enabled_nodes.is_empty() {
            return None;
        }

        // O(1) round-robin over enabled nodes
        let idx = self.round_robin_index % self.enabled_nodes.len();
        self.round_robin_index = self.round_robin_index.wrapping_add(1) % self.enabled_nodes.len();
        Some(self.enabled_nodes[idx].clone())
    }

    /// Manually disable a node (indefinite)
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

    /// Manually enable a node
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

    /// Auto-disable a node due to health check failure
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

    /// Auto-enable a node that recovered (only if it was auto-disabled)
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

    /// Update health check status for a node
    pub fn update_health_status(&mut self, addr: &str, status: HealthCheckStatus) {
        // O(1) HashMap lookup
        if let Some(node) = self.nodes.get_mut(addr) {
            node.last_health_check = Some(std::time::Instant::now());
            node.last_health_check_status = Some(status.clone());

            match status {
                HealthCheckStatus::Healthy => {
                    node.consecutive_failures = 0;
                }
                HealthCheckStatus::Unhealthy(_) => {
                    node.consecutive_failures += 1;
                }
            }
        }
    }

    /// Get consecutive failures for a node
    pub fn consecutive_failures(&self, addr: &str) -> u32 {
        // O(1) HashMap lookup
        self.nodes
            .get(addr)
            .map(|n| n.consecutive_failures)
            .unwrap_or(0)
    }

    /// Get all nodes (including disabled ones)
    pub fn all_nodes(&self) -> Vec<Node> {
        self.nodes.values().cloned().collect()
    }

    /// Get enabled nodes only
    pub fn enabled_nodes(&self) -> Vec<String> {
        // O(1) - just clone the enabled_nodes vec
        self.enabled_nodes.clone()
    }

    /// Get disabled nodes only
    pub fn disabled_nodes(&self) -> Vec<String> {
        // O(N) but this is only used for display purposes
        self.nodes
            .iter()
            .filter(|(_, n)| !n.enabled)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    /// Add a node to the pool
    pub fn add_node(&mut self, node_addr: String) {
        // O(1) HashMap contains check
        if !self.nodes.contains_key(&node_addr) {
            self.nodes.insert(node_addr.clone(), Node::new(node_addr.clone()));
            // New nodes are enabled by default
            self.enabled_nodes.push(node_addr);
        }
    }

    /// Remove a node from the pool
    pub fn remove_node(&mut self, node_addr: &str) {
        // O(1) HashMap remove
        self.nodes.remove(node_addr);
        // Also remove from enabled_nodes
        self.enabled_nodes.retain(|a| a != node_addr);
    }

    /// Get the number of nodes
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the number of enabled nodes
    pub fn enabled_count(&self) -> usize {
        // O(1) - just return the length of enabled_nodes
        self.enabled_nodes.len()
    }

    /// Get list of all node addresses (backward compatibility)
    pub fn nodes(&self) -> Vec<String> {
        // O(N) but this is only used for display
        self.nodes.keys().cloned().collect()
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
}
