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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Type of MaDRPC server
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerType {
    Node,
    Orchestrator,
}

/// Server information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub server_type: ServerType,
    pub version: String,
    pub uptime_ms: u64,
}

impl ServerInfo {
    pub fn new(server_type: ServerType, uptime_ms: u64) -> Self {
        Self {
            server_type,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_ms,
        }
    }
}

/// Metrics for a specific RPC method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMetrics {
    pub call_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_latency_us: u64,
    pub p50_latency_us: u64,
    pub p95_latency_us: u64,
    pub p99_latency_us: u64,
}

impl MethodMetrics {
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

/// Metrics for a specific node (used by orchestrator)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetrics {
    pub node_addr: String,
    pub request_count: u64,
    pub last_request_ms: u64,
}

impl NodeMetrics {
    pub fn new(node_addr: String) -> Self {
        Self {
            node_addr,
            request_count: 0,
            last_request_ms: 0,
        }
    }
}

/// Complete metrics snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub active_connections: u64,
    pub uptime_ms: u64,
    pub methods: HashMap<String, MethodMetrics>,
    pub nodes: Option<HashMap<String, NodeMetrics>>,
}

impl MetricsSnapshot {
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
