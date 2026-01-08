//! MaDRPC Orchestrator
//!
//! This crate provides the orchestrator (load balancer) component of the MaDRPC system.
//! The orchestrator sits between clients and compute nodes, forwarding RPC requests using
//! round-robin load balancing with circuit breaker pattern and periodic health checking.
//!
//! # Architecture
//!
//! The orchestrator is a "stupid forwarder" - it does NOT execute JavaScript code or have
//! a Boa engine. Its only responsibilities are:
//!
//! 1. **Load Balancing**: Distribute requests across nodes using round-robin selection
//! 2. **Circuit Breaking**: Prevent cascading failures by skipping unhealthy nodes
//! 3. **Health Checking**: Periodically verify node availability via TCP connections
//! 4. **Request Forwarding**: Forward requests to selected nodes and return responses
//!
//! # Key Design Decisions
//!
//! ## Connection-per-Request Strategy
//!
//! The orchestrator creates a new TCP connection for each request rather than maintaining
//! a connection pool. This design choice enables:
//!
//! - **True Parallelism**: Multiple requests to the same node execute concurrently
//! - **Simplified State Management**: No need to manage shared connection lifecycles
//! - **Fault Isolation**: Connection failures don't affect other requests
//!
//! The overhead of connection creation is acceptable because the orchestrator typically
//! runs on the same machine as nodes or in a low-latency network environment.
//!
//! ## Circuit Breaker Pattern
//!
//! Each node has a circuit breaker that transitions through three states:
//!
//! - **Closed**: Normal operation, requests flow through
//! - **Open**: Fail fast after consecutive failures exceed threshold
//! - **Half-Open**: Test recovery after exponential backoff timeout
//!
//! This prevents the orchestrator from wasting time on nodes that are likely to fail.
//!
//! ## Manual vs Auto Disable
//!
//! Nodes can be disabled in two ways with different semantics:
//!
//! - **Manual Disable**: User-initiated via API, never auto-re-enabled by health checker
//! - **Auto Disable**: Triggered by health check failures, can be auto-re-enabled on recovery
//!
//! # Example
//!
//! ```no_run
//! use madrpc_orchestrator::{Orchestrator, HealthCheckConfig};
//! use std::time::Duration;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create orchestrator with custom health check config
//! let health_config = HealthCheckConfig {
//!     interval: Duration::from_secs(10),
//!     timeout: Duration::from_millis(1000),
//!     failure_threshold: 5,
//! };
//!
//! let orchestrator = Orchestrator::with_config(
//!     vec![
//!         "127.0.0.1:9001".to_string(),
//!         "127.0.0.1:9002".to_string(),
//!     ],
//!     health_config,
//! ).await?;
//!
//! // Forward requests (round-robin with circuit breaking)
//! // let response = orchestrator.forward_request(&request).await?;
//!
//! // Manually manage nodes
//! orchestrator.disable_node("127.0.0.1:9001").await;
//! orchestrator.add_node("127.0.0.1:9003".to_string()).await;
//! # Ok(())
//! # }
//! ```

pub mod load_balancer;
pub mod node;
pub mod health_checker;
pub mod orchestrator;
pub mod http_router;
pub mod http_server;

pub use load_balancer::LoadBalancer;
pub use node::{Node, DisableReason, HealthCheckStatus, CircuitBreakerState, CircuitBreakerConfig};
pub use health_checker::{HealthChecker, HealthCheckConfig};
pub use orchestrator::{Orchestrator, RetryConfig};
pub use http_router::OrchestratorRouter;
pub use http_server::HttpServer;
