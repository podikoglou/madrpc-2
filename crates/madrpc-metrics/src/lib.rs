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

//! MaDRPC Metrics Collection
//!
//! This crate provides a thread-safe, high-performance metrics collection infrastructure
//! for MaDRPC nodes and orchestrators. It supports real-time monitoring of RPC calls,
//! latency tracking (with percentiles), request counting, and distributed system metrics.
//!
//! # Architecture
//!
//! The metrics system is built around three core components:
//!
//! - [`MetricsRegistry`]: Thread-safe storage for all metrics with lock-free counters
//! - [`MetricsCollector`]: Trait for collecting and handling metrics requests
//! - [`MetricsSnapshot`]: Serializable snapshot of current metrics state
//!
//! # Features
//!
//! - **Lock-free performance**: Uses atomic operations for hot path metrics collection
//! - **Latency percentiles**: Tracks P50, P95, P99 latencies using ring buffers
//! - **Built-in endpoints**: Special `_metrics` and `_info` methods for monitoring
//! - **Automatic cleanup**: LRU eviction with configurable TTLs to prevent memory bloat
//! - **Node tracking**: Orchestrator can track per-node request distribution
//!
//! # Usage Example
//!
//! ```rust
//! use madrpc_metrics::{NodeMetricsCollector, MetricsCollector};
//! use std::time::Instant;
//!
//! let collector = NodeMetricsCollector::new();
//!
//! // Record a successful RPC call
//! let start = Instant::now();
//! // ... execute RPC call ...
//! collector.record_call("my_method", start, true);
//!
//! // Get current metrics snapshot
//! let snapshot = collector.snapshot();
//! println!("Total requests: {}", snapshot.total_requests);
//! ```
//!
//! # Thread Safety
//!
//! All metrics collectors are thread-safe and can be shared across threads using `Arc`.
//! The registry uses a hybrid concurrency model:
//! - Lock-free atomics for counter increments (hot path)
//! - RwLock for metadata management (method/node registry access)
//!
//! # Built-in Monitoring Endpoints
//!
//! Both nodes and orchestrators automatically expose two special RPC methods:
//!
//! - **`_metrics`**: Returns complete `MetricsSnapshot` with all metrics
//! - **`_info`**: Returns `ServerInfo` with server type, version, and uptime
//!
//! These are intercepted by the metrics collector and never forwarded to actual handlers.

mod collector;
mod registry;
mod snapshot;

pub use collector::{MetricsCollector, NodeMetricsCollector, OrchestratorMetricsCollector};
pub use registry::MetricsRegistry;
pub use snapshot::{MethodMetrics, MetricsSnapshot, NodeMetrics, ServerInfo, ServerType};
