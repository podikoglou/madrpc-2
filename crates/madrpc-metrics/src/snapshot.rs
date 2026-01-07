use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Basic server information returned by the `_info` endpoint.
///
/// Provides metadata about a MaDRPC server including its type, version,
/// and uptime. This is useful for service discovery and monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// The type of server (Node or Orchestrator)
    pub server_type: ServerType,
    /// Version string from Cargo.toml
    pub version: String,
    /// Server uptime in milliseconds since start
    pub uptime_ms: u64,
}

impl ServerInfo {
    /// Creates a new server info struct.
    ///
    /// Automatically extracts the version from `CARGO_PKG_VERSION`.
    ///
    /// # Arguments
    /// * `server_type` - The type of server (Node or Orchestrator)
    /// * `uptime_ms` - The uptime in milliseconds since the server started
    ///
    /// # Example
    /// ```rust
    /// use madrpc_metrics::{ServerInfo, ServerType};
    ///
    /// let info = ServerInfo::new(ServerType::Node, 60000);
    /// assert_eq!(info.server_type, ServerType::Node);
    /// assert_eq!(info.uptime_ms, 60000);
    /// ```
    pub fn new(server_type: ServerType, uptime_ms: u64) -> Self {
        Self {
            server_type,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_ms,
        }
    }
}

/// Performance metrics for a specific RPC method.
///
/// Tracks call statistics and latency percentiles for individual methods.
/// Latency percentiles (P50, P95, P99) are computed from a rolling buffer
/// of recent samples, providing insight into tail latencies.
///
/// # Latency Tracking
///
/// - **avg_latency_us**: Mean latency across all recorded samples
/// - **p50_latency_us**: Median latency (50th percentile)
/// - **p95_latency_us**: 95th percentile latency
/// - **p99_latency_us**: 99th percentile latency
///
/// Note: Percentiles are calculated from up to 1000 most recent samples.
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
    ///
    /// All fields are initialized to zero. This is primarily used internally
    /// by the metrics registry.
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
    ///
    /// Initializes counters to zero. The address is preserved for identification.
    ///
    /// # Arguments
    /// * `node_addr` - The address of the node (e.g., "127.0.0.1:9001")
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
/// endpoint and can be serialized to JSON for monitoring systems.
///
/// # Fields
///
/// - **Global counters**: Total, successful, and failed request counts
/// - **Connection tracking**: Current active connections
/// - **Uptime**: Server uptime in milliseconds
/// - **Method metrics**: Per-method call statistics and latency percentiles
/// - **Node metrics**: (Orchestrator only) Per-node request distribution
///
/// # Example
///
/// ```rust
/// use madrpc_metrics::MetricsSnapshot;
/// use serde_json::json;
///
/// // Snapshot from _metrics endpoint
/// let snapshot: MetricsSnapshot = serde_json::from_value(json!({
///     "total_requests": 1000,
///     "successful_requests": 950,
///     "failed_requests": 50,
///     "active_connections": 10,
///     "uptime_ms": 60000,
///     "methods": {},
///     "nodes": null
/// })).unwrap();
///
/// let success_rate = snapshot.successful_requests as f64 / snapshot.total_requests as f64;
/// println!("Success rate: {:.2}%", success_rate * 100.0);
/// ```
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
    ///
    /// Initializes all counters to zero and empty collections.
    /// This is primarily used internally by the metrics registry.
    ///
    /// # Arguments
    /// * `uptime_ms` - The uptime in milliseconds
    /// * `include_nodes` - Whether to include node metrics (should be `true` for orchestrators)
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
