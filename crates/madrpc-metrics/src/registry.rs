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

use crate::snapshot::{MethodMetrics, MetricsSnapshot, NodeMetrics};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Instant, SystemTime};

const LATENCY_BUFFER_SIZE: usize = 1000;

/// Configuration for metrics cleanup behavior
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    /// Maximum number of unique methods to track
    pub max_methods: usize,
    /// Maximum number of unique nodes to track
    pub max_nodes: usize,
    /// Time-to-live for method entries in seconds
    pub method_ttl_secs: u64,
    /// Time-to-live for node entries in seconds
    pub node_ttl_secs: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            max_methods: 1000,
            max_nodes: 100,
            method_ttl_secs: 3600,
            node_ttl_secs: 3600,
        }
    }
}

/// Ring buffer for storing latency samples
#[derive(Debug)]
struct LatencyBuffer {
    samples: Vec<AtomicU64>,
    index: AtomicU64,
    count: AtomicU64,
}

impl LatencyBuffer {
    fn new() -> Self {
        Self {
            samples: (0..LATENCY_BUFFER_SIZE)
                .map(|_| AtomicU64::new(0))
                .collect(),
            index: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn record(&self, latency_us: u64) {
        let idx = self.index.fetch_add(1, Ordering::Relaxed) % LATENCY_BUFFER_SIZE as u64;
        self.samples[idx as usize].store(latency_us, Ordering::Relaxed);
        self.count.fetch_min(LATENCY_BUFFER_SIZE as u64, Ordering::Relaxed);
    }

    fn calculate_percentiles(&self) -> (u64, u64, u64, u64) {
        let mut samples: Vec<u64> = self
            .samples
            .iter()
            .map(|s| s.load(Ordering::Relaxed))
            .filter(|&s| s > 0)
            .collect();

        if samples.is_empty() {
            return (0, 0, 0, 0);
        }

        samples.sort_unstable();
        let len = samples.len();

        let avg = samples.iter().sum::<u64>() / len as u64;
        let p50 = samples[len * 50 / 100];
        let p95 = samples[len * 95 / 100];
        let p99 = samples[len * 99 / 100];

        (avg, p50, p95, p99)
    }
}

impl Default for LatencyBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-method metrics storage
#[derive(Debug)]
struct MethodStats {
    call_count: AtomicU64,
    success_count: AtomicU64,
    failure_count: AtomicU64,
    latencies: LatencyBuffer,
    last_access_ms: AtomicU64,
}

impl MethodStats {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        Self {
            call_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            latencies: LatencyBuffer::new(),
            last_access_ms: AtomicU64::new(now),
        }
    }

    fn update_last_access(&self) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.last_access_ms.store(now, Ordering::Relaxed);
    }

    fn increment_call(&self) {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        self.update_last_access();
    }

    fn increment_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
        self.update_last_access();
    }

    fn increment_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.update_last_access();
    }

    fn record_latency(&self, latency_us: u64) {
        self.latencies.record(latency_us);
        self.update_last_access();
    }

    fn snapshot(&self) -> MethodMetrics {
        let call_count = self.call_count.load(Ordering::Relaxed);
        let success_count = self.success_count.load(Ordering::Relaxed);
        let failure_count = self.failure_count.load(Ordering::Relaxed);
        let (avg_latency_us, p50_latency_us, p95_latency_us, p99_latency_us) =
            self.latencies.calculate_percentiles();

        MethodMetrics {
            call_count,
            success_count,
            failure_count,
            avg_latency_us,
            p50_latency_us,
            p95_latency_us,
            p99_latency_us,
        }
    }
}

impl Default for MethodStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-node metrics storage (for orchestrator)
#[derive(Debug)]
struct NodeStats {
    request_count: AtomicU64,
    last_request_ms: AtomicU64,
}

impl NodeStats {
    fn new() -> Self {
        Self {
            request_count: AtomicU64::new(0),
            last_request_ms: AtomicU64::new(0),
        }
    }

    fn record_request(&self) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.last_request_ms.store(now, Ordering::Relaxed);
    }

    fn snapshot(&self, node_addr: String) -> NodeMetrics {
        let request_count = self.request_count.load(Ordering::Relaxed);
        let last_request_ms = self.last_request_ms.load(Ordering::Relaxed);

        NodeMetrics {
            node_addr,
            request_count,
            last_request_ms,
        }
    }
}

impl Default for NodeStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe metrics registry with lock-free operations
#[derive(Debug)]
pub struct MetricsRegistry {
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    active_connections: AtomicU64,
    methods: StdRwLock<HashMap<String, Arc<MethodStats>>>,
    nodes: StdRwLock<HashMap<String, Arc<NodeStats>>>,
    start_time: Instant,
    config: MetricsConfig,
    cleanup_counter: AtomicU64,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self::with_config(MetricsConfig::default())
    }

    pub fn with_config(config: MetricsConfig) -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            methods: StdRwLock::new(HashMap::new()),
            nodes: StdRwLock::new(HashMap::new()),
            start_time: Instant::now(),
            config,
            cleanup_counter: AtomicU64::new(0),
        }
    }

    pub fn increment_total(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_success(&self) {
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_failure(&self) {
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_active_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_active_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record a method call with latency
    pub fn record_method_call(&self, method: &str, latency_us: u64, success: bool) {
        // Increment global counters
        self.increment_total();
        if success {
            self.increment_success();
        } else {
            self.increment_failure();
        }

        // Get or create method stats
        {
            let mut methods = self.methods.write().unwrap();
            let stats = methods
                .entry(method.to_string())
                .or_insert_with(|| Arc::new(MethodStats::new()))
                .clone();

            // Release the lock before recording
            drop(methods);

            stats.increment_call();
            stats.record_latency(latency_us);

            if success {
                stats.increment_success();
            } else {
                stats.increment_failure();
            }
        }
    }

    /// Record a request to a specific node (for orchestrator)
    pub fn record_node_request(&self, node_addr: &str) {
        let stats = {
            let mut nodes = self.nodes.write().unwrap();
            nodes
                .entry(node_addr.to_string())
                .or_insert_with(|| Arc::new(NodeStats::new()))
                .clone()
        };

        stats.record_request();
    }

    /// Get uptime in milliseconds
    pub fn uptime_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Take a snapshot of current metrics
    pub fn snapshot(&self, include_nodes: bool) -> MetricsSnapshot {
        let uptime_ms = self.uptime_ms();
        let total_requests = self.total_requests.load(Ordering::Relaxed);
        let successful_requests = self.successful_requests.load(Ordering::Relaxed);
        let failed_requests = self.failed_requests.load(Ordering::Relaxed);
        let active_connections = self.active_connections.load(Ordering::Relaxed);

        let methods: HashMap<String, MethodMetrics> = {
            let methods_guard = self.methods.read().unwrap();
            methods_guard
                .iter()
                .map(|(name, stats)| (name.clone(), stats.snapshot()))
                .collect()
        };

        let nodes = if include_nodes {
            let nodes_guard = self.nodes.read().unwrap();
            Some(
                nodes_guard
                    .iter()
                    .map(|(addr, stats)| (addr.clone(), stats.snapshot(addr.clone())))
                    .collect(),
            )
        } else {
            None
        };

        MetricsSnapshot {
            total_requests,
            successful_requests,
            failed_requests,
            active_connections,
            uptime_ms,
            methods,
            nodes,
        }
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_counter_increments() {
        let registry = MetricsRegistry::new();

        registry.increment_total();
        registry.increment_total();
        registry.increment_success();
        registry.increment_failure();

        let snapshot = registry.snapshot(false);
        assert_eq!(snapshot.total_requests, 2);
        assert_eq!(snapshot.successful_requests, 1);
        assert_eq!(snapshot.failed_requests, 1);
    }

    #[test]
    fn test_active_connections() {
        let registry = MetricsRegistry::new();

        registry.increment_active_connections();
        registry.increment_active_connections();
        registry.increment_active_connections();

        assert_eq!(registry.snapshot(false).active_connections, 3);

        registry.decrement_active_connections();
        assert_eq!(registry.snapshot(false).active_connections, 2);
    }

    #[test]
    fn test_method_tracking() {
        let registry = MetricsRegistry::new();

        // Record some calls
        registry.record_method_call("test_method", 100, true);
        registry.record_method_call("test_method", 200, true);
        registry.record_method_call("test_method", 50, false);

        let snapshot = registry.snapshot(false);
        assert_eq!(snapshot.total_requests, 3);
        assert_eq!(snapshot.successful_requests, 2);
        assert_eq!(snapshot.failed_requests, 1);

        let method_metrics = snapshot.methods.get("test_method").unwrap();
        assert_eq!(method_metrics.call_count, 3);
        assert_eq!(method_metrics.success_count, 2);
        assert_eq!(method_metrics.failure_count, 1);
        assert_eq!(method_metrics.avg_latency_us, 116); // (100 + 200 + 50) / 3
    }

    #[test]
    fn test_percentile_calculation() {
        let registry = MetricsRegistry::new();

        // Record many latency samples
        for i in 0..1000 {
            registry.record_method_call("percentile_test", i, true);
        }

        let snapshot = registry.snapshot(false);
        let method_metrics = snapshot.methods.get("percentile_test").unwrap();

        // P50 should be around 500
        assert!(method_metrics.p50_latency_us >= 400 && method_metrics.p50_latency_us <= 600);

        // P95 should be around 950
        assert!(
            method_metrics.p95_latency_us >= 900 && method_metrics.p95_latency_us <= 999
        );

        // P99 should be around 990
        assert!(
            method_metrics.p99_latency_us >= 980 && method_metrics.p99_latency_us <= 999
        );
    }

    #[test]
    fn test_node_request_tracking() {
        let registry = MetricsRegistry::new();

        registry.record_node_request("node1");
        registry.record_node_request("node1");
        registry.record_node_request("node2");

        let snapshot = registry.snapshot(true);
        assert!(snapshot.nodes.is_some());

        let nodes = snapshot.nodes.unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes.get("node1").unwrap().request_count, 2);
        assert_eq!(nodes.get("node2").unwrap().request_count, 1);
    }

    #[test]
    fn test_thread_safety() {
        let registry = Arc::new(MetricsRegistry::new());
        let mut handles = vec![];

        // Spawn multiple threads incrementing counters
        for _ in 0..10 {
            let registry_clone = registry.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    registry_clone.record_method_call("concurrent_test", 100, true);
                }
            }));
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        let snapshot = registry.snapshot(false);
        assert_eq!(snapshot.total_requests, 10000);
        assert_eq!(snapshot.successful_requests, 10000);

        let method_metrics = snapshot.methods.get("concurrent_test").unwrap();
        assert_eq!(method_metrics.call_count, 10000);
    }

    #[test]
    fn test_uptime() {
        let registry = MetricsRegistry::new();
        thread::sleep(Duration::from_millis(10));

        let uptime = registry.uptime_ms();
        assert!(uptime >= 10);
    }
}

