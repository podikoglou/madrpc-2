//! Built-in procedure response types
//!
//! This module defines strongly-typed response schemas for all built-in RPC procedures
//! (`_metrics`, `_info`, `_health`). These types provide compile-time type safety
//! and serve as the single source of truth for the wire format of these endpoints.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Server Type
// ============================================================================

/// Type of MaDRPC server.
///
/// Distinguishes between compute nodes (which execute JavaScript) and
/// orchestrators (which load balance requests).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerType {
    /// A compute node that executes JavaScript functions
    Node,
    /// An orchestrator that load balances requests across nodes
    Orchestrator,
}

// ============================================================================
// Health Response
// ============================================================================

/// Health check response returned by the `_health` endpoint.
///
/// A simple health check that indicates the server is running and able to respond.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    /// Status of the server - always "healthy" when the endpoint responds
    pub status: String,
}

impl HealthResponse {
    /// Creates a new healthy response.
    ///
    /// # Example
    /// ```rust
    /// use madrpc_common::protocol::builtin::HealthResponse;
    ///
    /// let response = HealthResponse::healthy();
    /// assert_eq!(response.status, "healthy");
    /// ```
    pub fn healthy() -> Self {
        Self {
            status: "healthy".to_string(),
        }
    }
}

// ============================================================================
// Metrics Response Types
// ============================================================================

/// Performance metrics for a specific RPC method.
///
/// Tracks call statistics and latency percentiles for individual methods.
/// Latency percentiles (P50, P95, P99) are computed from a rolling buffer
/// of recent samples, providing insight into tail latencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMetrics {
    /// Total number of times this method was called
    pub call_count: u64,
    /// Number of successful calls (no errors)
    pub success_count: u64,
    /// Number of failed calls (errors thrown)
    pub failure_count: u64,
    /// Average latency in microseconds
    pub avg_latency_us: u64,
    /// Median latency (50th percentile) in microseconds
    pub p50_latency_us: u64,
    /// 95th percentile latency in microseconds
    pub p95_latency_us: u64,
    /// 99th percentile latency in microseconds
    pub p99_latency_us: u64,
}

impl MethodMetrics {
    /// Creates a new empty method metrics struct.
    pub fn new() -> Self {
        Self {
            call_count: 0,
            success_count: 0,
            failure_count: 0,
            avg_latency_us: 0,
            p50_latency_us: 0,
            p95_latency_us: 0,
            p99_latency_us: 0,
        }
    }
}

impl Default for MethodMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics for a specific node tracked by an orchestrator.
///
/// Orchestrators track how many requests are forwarded to each node
/// and when the last request was sent. This helps monitor load balancer
/// behavior and detect stale nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetrics {
    /// The address of the node (e.g., "127.0.0.1:9001")
    pub node_addr: String,
    /// Total number of requests forwarded to this node
    pub request_count: u64,
    /// Unix timestamp (ms) of the last request to this node
    pub last_request_ms: u64,
}

impl NodeMetrics {
    /// Creates a new node metrics struct.
    pub fn new(node_addr: String) -> Self {
        Self {
            node_addr,
            request_count: 0,
            last_request_ms: 0,
        }
    }
}

/// Complete snapshot of all metrics at a point in time.
///
/// `MetricsSnapshot` provides a comprehensive view of server performance,
/// including global counters, per-method statistics, and (for orchestrators)
/// per-node request distribution. Snapshots are returned by the `_metrics`
/// endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Total number of RPC requests processed
    pub total_requests: u64,
    /// Number of successfully completed requests
    pub successful_requests: u64,
    /// Number of failed requests
    pub failed_requests: u64,
    /// Current number of active TCP connections
    pub active_connections: u64,
    /// Server uptime in milliseconds
    pub uptime_ms: u64,
    /// Per-method metrics keyed by method name
    pub methods: HashMap<String, MethodMetrics>,
    /// Per-node metrics (only present for orchestrators)
    pub nodes: Option<HashMap<String, NodeMetrics>>,
}

impl MetricsSnapshot {
    /// Creates a new empty metrics snapshot.
    pub fn new(uptime_ms: u64, include_nodes: bool) -> Self {
        Self {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            active_connections: 0,
            uptime_ms,
            methods: HashMap::new(),
            nodes: if include_nodes {
                Some(HashMap::new())
            } else {
                None
            },
        }
    }
}

/// Type alias for metrics response - used as the return type for `_metrics` endpoint.
pub type MetricsResponse = MetricsSnapshot;

// ============================================================================
// Info Response Types
// ============================================================================

/// Base server information shared by both nodes and orchestrators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerBase {
    /// The type of server (Node or Orchestrator)
    pub server_type: ServerType,
    /// Version string from Cargo.toml
    pub version: String,
    /// Server uptime in milliseconds since start
    pub uptime_ms: u64,
}

impl ServerBase {
    /// Creates a new server base info struct.
    pub fn new(server_type: ServerType, uptime_ms: u64) -> Self {
        Self {
            server_type,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_ms,
        }
    }
}

/// Node info returned by the `_info` endpoint for nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Version string
    pub version: String,
    /// Server uptime in milliseconds since start
    pub uptime_ms: u64,
}

impl NodeInfo {
    /// Creates a new node info struct.
    pub fn new(uptime_ms: u64) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_ms,
        }
    }
}

/// Information about a single node in an orchestrator's pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorNodeInfo {
    /// The address of the node
    pub addr: String,
    /// Whether this node is enabled for request routing
    pub enabled: bool,
    /// Reason why this node is disabled (if applicable)
    pub disable_reason: Option<String>,
    /// Current state of the circuit breaker
    pub circuit_state: String,
    /// Number of consecutive failures
    pub consecutive_failures: u32,
}

/// Orchestrator info returned by the `_info` endpoint for orchestrators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorInfo {
    /// Version string
    pub version: String,
    /// Server uptime in milliseconds since start
    pub uptime_ms: u64,
    /// Total number of nodes in the pool
    pub total_nodes: usize,
    /// Number of enabled nodes
    pub enabled_nodes: usize,
    /// Number of disabled nodes
    pub disabled_nodes: usize,
    /// Information about each node in the pool
    pub nodes: Vec<OrchestratorNodeInfo>,
}

impl OrchestratorInfo {
    /// Creates a new orchestrator info struct.
    pub fn new(uptime_ms: u64) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_ms,
            total_nodes: 0,
            enabled_nodes: 0,
            disabled_nodes: 0,
            nodes: Vec::new(),
        }
    }
}

/// Unified info response with discriminated enum for node and orchestrator variants.
///
/// This uses serde's "internally tagged" enum representation with `server_type` as the tag,
/// ensuring proper serialization/deserialization between the two variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "server_type", rename_all = "lowercase")]
pub enum InfoResponse {
    /// Node variant - returned by compute nodes
    Node(NodeInfo),
    /// Orchestrator variant - returned by orchestrators
    Orchestrator(OrchestratorInfo),
}
