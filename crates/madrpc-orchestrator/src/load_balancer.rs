use std::collections::VecDeque;

/// Round-robin load balancer for nodes
pub struct LoadBalancer {
    nodes: VecDeque<String>,
}

impl LoadBalancer {
    /// Create a new load balancer with a static node list
    pub fn new(nodes: Vec<String>) -> Self {
        Self {
            nodes: VecDeque::from(nodes),
        }
    }

    /// Get the next node using round-robin
    pub fn next_node(&mut self) -> Option<String> {
        if self.nodes.is_empty() {
            return None;
        }

        // Rotate: move first to back, return it
        let node = self.nodes.pop_front()?;
        self.nodes.push_back(node.clone());
        Some(node)
    }

    /// Add a node to the pool
    pub fn add_node(&mut self, node: String) {
        if !self.nodes.contains(&node) {
            self.nodes.push_back(node);
        }
    }

    /// Remove a node from the pool
    pub fn remove_node(&mut self, node: &str) {
        self.nodes.retain(|n| n != node);
    }

    /// Get the number of nodes
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get list of all nodes
    pub fn nodes(&self) -> Vec<String> {
        self.nodes.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_balancer_creation() {
        let nodes = vec![
            "localhost:9001".to_string(),
            "localhost:9002".to_string(),
        ];
        let lb = LoadBalancer::new(nodes.clone());
        assert_eq!(lb.node_count(), 2);
    }

    #[test]
    fn test_round_robin() {
        let nodes = vec![
            "node1".to_string(),
            "node2".to_string(),
            "node3".to_string(),
        ];
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
        // duplicate
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
        assert_eq!(
            lb.nodes(),
            vec!["node1".to_string(), "node3".to_string()]
        );
    }

    #[test]
    fn test_get_nodes() {
        let nodes = vec!["a".to_string(), "b".to_string()];
        let lb = LoadBalancer::new(nodes.clone());
        assert_eq!(lb.nodes(), nodes);
    }
}
