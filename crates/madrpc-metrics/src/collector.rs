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

use crate::registry::MetricsRegistry;
use crate::snapshot::{MetricsSnapshot, ServerInfo, ServerType};
use madrpc_common::protocol::{Request, Response};
use madrpc_common::protocol::error::Result as MadrpcResult;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

/// Trait for metrics collection
pub trait MetricsCollector: Send + Sync {
    /// Check if this is a metrics/info request that should not be forwarded
    fn is_metrics_request(&self, method: &str) -> bool;

    /// Handle a metrics/info request
    fn handle_metrics_request(&self, method: &str, id: u64) -> MadrpcResult<Response>;

    /// Record a method call
    fn record_call(&self, method: &str, start_time: Instant, success: bool);

    /// Get current metrics snapshot
    fn snapshot(&self) -> MetricsSnapshot;
}

/// Metrics collector for Node
pub struct NodeMetricsCollector {
    registry: Arc<MetricsRegistry>,
}

impl NodeMetricsCollector {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::new()),
        }
    }

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

/// Metrics collector for Orchestrator
pub struct OrchestratorMetricsCollector {
    registry: Arc<MetricsRegistry>,
}

impl OrchestratorMetricsCollector {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::new()),
        }
    }

    pub fn with_registry(registry: Arc<MetricsRegistry>) -> Self {
        Self { registry }
    }

    /// Record that a request was forwarded to a specific node
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
