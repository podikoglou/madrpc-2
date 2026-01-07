// Copyright 2025 MaDRPC Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::registry::{MetricsConfig, MetricsRegistry};
use crate::snapshot::{MetricsSnapshot, ServerInfo, ServerType};
use madrpc_common::protocol::Response;
use madrpc_common::protocol::error::Result as MadrpcResult;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

/// Trait for metrics collection in MaDRPC servers.
///
/// This trait defines the interface for collecting and reporting metrics in both
/// nodes and orchestrators. Implementations handle built-in monitoring endpoints
/// (`_metrics` and `_info`) and track RPC call statistics.
///
/// # Built-in Methods
///
/// Implementations recognize two special RPC methods that are intercepted
/// before reaching user handlers:
///
/// - `_metrics`: Returns a complete `MetricsSnapshot` as JSON
/// - `_info`: Returns a `ServerInfo` struct with server metadata
///
/// # Example
///
/// ```rust
/// use madrpc_metrics::{NodeMetricsCollector, MetricsCollector};
/// use std::time::Instant;
///
/// let collector = NodeMetricsCollector::new();
///
/// // Check if a request is for metrics
/// if collector.is_metrics_request("_metrics") {
///     // Handle the metrics request
///     let response = collector.handle_metrics_request("_metrics", 123).unwrap();
/// }
///
/// // Record an RPC call
/// let start = Instant::now();
/// // ... execute method ...
/// collector.record_call("my_method", start, true);
/// ```
pub trait MetricsCollector: Send + Sync {
    /// Checks if a method name is a built-in metrics endpoint.
    ///
    /// Returns `true` for `_metrics` and `_info` requests, which should be
    /// intercepted and handled by the metrics collector rather than being
    /// forwarded to user-defined RPC handlers.
    ///
    /// # Arguments
    /// * `method` - The method name to check
    ///
    /// # Returns
    /// `true` if this is a metrics/info request that should not be forwarded
    fn is_metrics_request(&self, method: &str) -> bool;

    /// Handles a metrics or info request and returns a response.
    ///
    /// This method generates responses for the built-in `_metrics` and `_info`
    /// endpoints. It should only be called after `is_metrics_request()` returns
    /// `true` for the given method.
    ///
    /// # Arguments
    /// * `method` - The method name (must be `_metrics` or `_info`)
    /// * `id` - The request ID to include in the response
    ///
    /// # Returns
    /// A `Response` containing either:
    /// - `MetricsSnapshot` (for `_metrics`)
    /// - `ServerInfo` (for `_info`)
    ///
    /// # Panics
    /// May panic if called with a method name that is not a metrics endpoint
    fn handle_metrics_request(&self, method: &str, id: u64) -> MadrpcResult<Response>;

    /// Records an RPC method call with its outcome and latency.
    ///
    /// This method updates internal counters and latency tracking for the
    /// specified method. It should be called after every RPC call completes.
    ///
    /// # Arguments
    /// * `method` - The name of the method that was called
    /// * `start_time` - The `Instant` when the call began (for latency calculation)
    /// * `success` - `true` if the call succeeded, `false` if it failed
    ///
    /// # Example
    /// ```rust
    /// use std::time::Instant;
    /// let start = Instant::now();
    /// // ... execute RPC call ...
    /// let success = true; // or false if it failed
    /// collector.record_call("my_method", start, success);
    /// ```
    fn record_call(&self, method: &str, start_time: Instant, success: bool);

    /// Takes a snapshot of the current metrics state.
    ///
    /// Returns a `MetricsSnapshot` containing all current metrics including
    /// request counts, latency percentiles, and (for orchestrators) per-node
    /// statistics.
    ///
    /// # Returns
    /// A snapshot of all current metrics at the time of the call
    fn snapshot(&self) -> MetricsSnapshot;
}

/// Metrics collector for MaDRPC nodes.
///
/// `NodeMetricsCollector` tracks RPC method execution statistics on compute nodes,
/// including call counts, success/failure rates, and latency percentiles (P50, P95, P99).
///
/// # Key Differences from OrchestratorMetricsCollector
///
/// - **No node tracking**: Nodes don't track downstream node metrics
/// - **ServerType::Node**: Reports itself as a node in `_info` endpoint
/// - **Simpler snapshots**: Metrics snapshots don't include node distribution data
///
/// # Example
///
/// ```rust
/// use madrpc_metrics::{NodeMetricsCollector, MetricsCollector};
/// use std::time::Instant;
///
/// let collector = NodeMetricsCollector::new();
///
/// // Record some method calls
/// let start = Instant::now();
/// collector.record_call("compute_pi", start, true);
/// collector.record_call("compute_pi", start, false);
///
/// // Get metrics
/// let snapshot = collector.snapshot();
/// assert_eq!(snapshot.total_requests, 2);
/// ```
pub struct NodeMetricsCollector {
    registry: Arc<MetricsRegistry>,
}

impl NodeMetricsCollector {
    /// Creates a new node metrics collector with default configuration.
    ///
    /// Uses default `MetricsConfig` values:
    /// - `max_methods`: 1000
    /// - `max_nodes`: 100
    /// - `method_ttl_secs`: 3600 (1 hour)
    /// - `node_ttl_secs`: 3600 (1 hour)
    pub fn new() -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::new()),
        }
    }

    /// Creates a new node metrics collector with custom configuration.
    ///
    /// Use this to control memory usage and cleanup behavior for method metrics.
    ///
    /// # Arguments
    /// * `config` - The metrics configuration controlling limits and TTLs
    ///
    /// # Example
    /// ```rust
    /// use madrpc_metrics::{NodeMetricsCollector, MetricsConfig};
    ///
    /// let config = MetricsConfig {
    ///     max_methods: 100,
    ///     max_nodes: 10,
    ///     method_ttl_secs: 300,  // 5 minutes
    ///     node_ttl_secs: 300,
    /// };
    /// let collector = NodeMetricsCollector::with_config(config);
    /// ```
    pub fn with_config(config: MetricsConfig) -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::with_config(config)),
        }
    }

    /// Creates a new node metrics collector with a custom registry.
    ///
    /// This allows sharing a registry between multiple collectors or using
    /// a pre-configured registry. Useful for testing or custom metrics setups.
    ///
    /// # Arguments
    /// * `registry` - The metrics registry to use
    pub fn with_registry(registry: Arc<MetricsRegistry>) -> Self {
        Self { registry }
    }
}

impl MetricsCollector for NodeMetricsCollector {
    fn is_metrics_request(&self, method: &str) -> bool {
        method == "_metrics" || method == "_info"
    }

    fn handle_metrics_request(&self, method: &str, id: u64) -> MadrpcResult<Response> {
        if method == "_metrics" {
            let snapshot = self.snapshot();
            Ok(Response::success(id, json!(snapshot)))
        } else if method == "_info" {
            let uptime_ms = self.registry.uptime_ms();
            let info = ServerInfo::new(ServerType::Node, uptime_ms);
            Ok(Response::success(id, json!(info)))
        } else {
            unreachable!("is_metrics_request should be called first");
        }
    }

    fn record_call(&self, method: &str, start_time: Instant, success: bool) {
        let latency_us = start_time.elapsed().as_micros() as u64;
        self.registry.record_method_call(method, latency_us, success);
    }

    fn snapshot(&self) -> MetricsSnapshot {
        self.registry.snapshot(false) // Nodes don't track other nodes
    }
}

/// Metrics collector for MaDRPC orchestrators.
///
/// `OrchestratorMetricsCollector` tracks request statistics at the orchestrator level,
/// including method call metrics and per-node request distribution. This enables monitoring
/// of load balancing behavior and downstream node health.
///
/// # Key Features
///
/// - **Per-node tracking**: Records how many requests are forwarded to each node
/// - **Request distribution**: Monitors load balancer behavior
/// - **Enhanced snapshots**: Includes node-level metrics in snapshots
///
/// # Example
///
/// ```rust
/// use madrpc_metrics::OrchestratorMetricsCollector;
///
/// let collector = OrchestratorMetricsCollector::new();
///
/// // Track which nodes receive requests
/// collector.record_node_request("127.0.0.1:9001");
/// collector.record_node_request("127.0.0.1:9002");
///
/// // Get metrics including node distribution
/// let snapshot = collector.snapshot();
/// if let Some(nodes) = snapshot.nodes {
///     for (addr, metrics) in nodes {
///         println!("{}: {} requests", addr, metrics.request_count);
///     }
/// }
/// ```
pub struct OrchestratorMetricsCollector {
    registry: Arc<MetricsRegistry>,
}

impl OrchestratorMetricsCollector {
    /// Creates a new orchestrator metrics collector with default configuration.
    ///
    /// Uses default `MetricsConfig` values:
    /// - `max_methods`: 1000
    /// - `max_nodes`: 100
    /// - `method_ttl_secs`: 3600 (1 hour)
    /// - `node_ttl_secs`: 3600 (1 hour)
    pub fn new() -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::new()),
        }
    }

    /// Creates a new orchestrator metrics collector with custom configuration.
    ///
    /// Use this to control memory usage and cleanup behavior for both method
    /// and node metrics. The `max_nodes` setting is particularly important
    /// for orchestrators tracking many downstream nodes.
    ///
    /// # Arguments
    /// * `config` - The metrics configuration controlling limits and TTLs
    pub fn with_config(config: MetricsConfig) -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::with_config(config)),
        }
    }

    /// Creates a new orchestrator metrics collector with a custom registry.
    ///
    /// This allows sharing a registry between multiple collectors or using
    /// a pre-configured registry. Useful for testing or custom metrics setups.
    ///
    /// # Arguments
    /// * `registry` - The metrics registry to use
    pub fn with_registry(registry: Arc<MetricsRegistry>) -> Self {
        Self { registry }
    }

    /// Records that a request was forwarded to a specific node.
    ///
    /// This method tracks request distribution across nodes, enabling monitoring
    /// of load balancer behavior and node utilization. Call this each time the
    /// orchestrator forwards a request to a node.
    ///
    /// # Arguments
    /// * `node_addr` - The address of the node that received the request
    ///
    /// # Example
    /// ```rust
    /// // In orchestrator request handling
    /// let selected_node = load_balancer.next_node();
    /// collector.record_node_request(&selected_node.addr);
    /// // ... forward request to selected_node ...
    /// ```
    pub fn record_node_request(&self, node_addr: &str) {
        self.registry.record_node_request(node_addr);
    }
}

impl MetricsCollector for OrchestratorMetricsCollector {
    fn is_metrics_request(&self, method: &str) -> bool {
        method == "_metrics" || method == "_info"
    }

    fn handle_metrics_request(&self, method: &str, id: u64) -> MadrpcResult<Response> {
        if method == "_metrics" {
            let snapshot = self.snapshot();
            Ok(Response::success(id, json!(snapshot)))
        } else if method == "_info" {
            let uptime_ms = self.registry.uptime_ms();
            let info = ServerInfo::new(ServerType::Orchestrator, uptime_ms);
            Ok(Response::success(id, json!(info)))
        } else {
            unreachable!("is_metrics_request should be called first");
        }
    }

    fn record_call(&self, method: &str, start_time: Instant, success: bool) {
        let latency_us = start_time.elapsed().as_micros() as u64;
        self.registry.record_method_call(method, latency_us, success);
    }

    fn snapshot(&self) -> MetricsSnapshot {
        self.registry.snapshot(true) // Orchestrator tracks nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_metrics_collector() {
        let collector = NodeMetricsCollector::new();

        // Test is_metrics_request
        assert!(collector.is_metrics_request("_metrics"));
        assert!(collector.is_metrics_request("_info"));
        assert!(!collector.is_metrics_request("other"));

        // Test snapshot
        let snapshot = collector.snapshot();
        assert_eq!(snapshot.total_requests, 0);
        assert!(snapshot.nodes.is_none());

        // Test info request
        let response = collector.handle_metrics_request("_info", 1).unwrap();
        assert!(response.success);
        assert!(response.result.is_some());
    }

    #[test]
    fn test_orchestrator_metrics_collector() {
        let collector = OrchestratorMetricsCollector::new();

        // Test is_metrics_request
        assert!(collector.is_metrics_request("_metrics"));
        assert!(collector.is_metrics_request("_info"));
        assert!(!collector.is_metrics_request("other"));

        // Test snapshot
        let snapshot = collector.snapshot();
        assert_eq!(snapshot.total_requests, 0);
        assert!(snapshot.nodes.is_some());

        // Test node request recording
        collector.record_node_request("node1");
        collector.record_node_request("node1");
        collector.record_node_request("node2");

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.nodes.as_ref().unwrap().len(), 2);
        assert_eq!(
            snapshot
                .nodes
                .as_ref()
                .unwrap()
                .get("node1")
                .unwrap()
                .request_count,
            2
        );
    }
}
