use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::{Result, MadrpcError};
use madrpc_common::transport::TcpTransportAsync;
use madrpc_metrics::{MetricsCollector, OrchestratorMetricsCollector};
use crate::load_balancer::LoadBalancer;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

/// MaDRPC Orchestrator - "stupid" forwarder
///
/// The orchestrator receives requests from clients, uses the load balancer
/// to pick a node, forwards the request to that node, and returns the
/// node's response to the client.
///
/// IMPORTANT: The orchestrator does NOT have a QuickJS engine. It only forwards.
pub struct Orchestrator {
    load_balancer: Arc<RwLock<LoadBalancer>>,
    node_connections: Arc<RwLock<HashMap<String, Arc<Mutex<tokio::net::TcpStream>>>>>,
    transport: TcpTransportAsync,
    metrics_collector: Arc<OrchestratorMetricsCollector>,
}

impl Orchestrator {
    /// Create a new orchestrator with static node list
    pub async fn new(node_addrs: Vec<String>) -> Result<Self> {
        let load_balancer = Arc::new(RwLock::new(LoadBalancer::new(node_addrs)));
        let node_connections = Arc::new(RwLock::new(HashMap::new()));
        let transport = TcpTransportAsync::new()?;

        // Initialize metrics
        let metrics_collector = Arc::new(OrchestratorMetricsCollector::new());

        Ok(Self {
            load_balancer,
            node_connections,
            transport,
            metrics_collector,
        })
    }

    /// Forward a request to the next available node
    ///
    /// This method:
    /// 1. Gets the next node via round-robin from the load balancer
    /// 2. Gets or creates a connection to that node
    /// 3. Forwards the request to the node
    /// 4. Returns the node's response
    pub async fn forward_request(&self, request: &Request) -> Result<Response> {
        // Check for metrics/info requests (do NOT forward these)
        if self.metrics_collector.is_metrics_request(&request.method) {
            return self
                .metrics_collector
                .handle_metrics_request(&request.method, request.id);
        }

        let start_time = Instant::now();
        let method = request.method.clone();

        // Get next node via round-robin
        let node_addr = {
            let mut lb = self.load_balancer.write().await;
            lb.next_node().ok_or(MadrpcError::AllNodesFailed)?
        };

        // Track which node received the request
        self.metrics_collector.record_node_request(&node_addr);

        // Get or create connection to node
        let conn = self.get_node_connection(&node_addr).await?;

        // Forward request
        // Note: We need to lock the mutex to get mutable access to the stream
        let response = {
            let mut stream = conn.lock().await;
            self.transport.send_request(&mut stream, request).await?
        };

        // Record metrics based on response
        let success = response.success;
        self.metrics_collector.record_call(&method, start_time, success);

        Ok(response)
    }

    /// Get or create a connection to a node
    ///
    /// This method caches connections in a HashMap for reuse.
    /// If a connection exists, it returns the cached connection.
    /// Otherwise, it creates a new connection and caches it.
    async fn get_node_connection(&self, node_addr: &str) -> Result<Arc<Mutex<tokio::net::TcpStream>>> {
        // Check if we have an existing connection
        {
            let conns = self.node_connections.read().await;
            if let Some(conn) = conns.get(node_addr) {
                // For TCP, we can't easily check if the connection is still valid
                // without trying to use it. We'll just return the cached connection
                // and handle errors during send_request.
                return Ok(conn.clone());
            }
        }

        // Create new connection
        let stream = self.transport.connect(node_addr).await?;

        // Wrap it in Arc<Mutex<>> for shared mutable access
        let conn = Arc::new(Mutex::new(stream));

        // Cache it
        let mut conns = self.node_connections.write().await;
        conns.insert(node_addr.to_string(), conn.clone());

        Ok(conn)
    }

    /// Add a node to the load balancer
    pub async fn add_node(&self, node_addr: String) {
        let mut lb = self.load_balancer.write().await;
        lb.add_node(node_addr);
    }

    /// Remove a node from the load balancer
    ///
    /// This also removes the cached connection to the node.
    /// Note: TCP connections will be closed when the Arc<Mutex<TcpStream>> is dropped.
    pub async fn remove_node(&self, node_addr: &str) {
        let mut lb = self.load_balancer.write().await;
        lb.remove_node(node_addr);

        // Also remove connection
        let mut conns = self.node_connections.write().await;
        conns.remove(node_addr);
        // The TCP connection will be automatically closed when the Arc<Mutex<TcpStream>>
        // is dropped (there are no more references to it)
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
        let nodes = vec![
            "localhost:9001".to_string(),
            "localhost:9002".to_string(),
        ];
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
        let orch = Orchestrator::new(vec!["node1".to_string()]).await.unwrap();
        orch.remove_node("node1").await;
        assert_eq!(orch.node_count().await, 0);
    }

    #[tokio::test]
    async fn test_orchestrator_nodes() {
        let nodes = vec![
            "node1".to_string(),
            "node2".to_string(),
        ];
        let orch = Orchestrator::new(nodes.clone()).await.unwrap();
        assert_eq!(orch.nodes().await, nodes);
    }

    #[tokio::test]
    async fn test_orchestrator_add_duplicate_node() {
        let orch = Orchestrator::new(vec!["node1".to_string()]).await.unwrap();
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
}
