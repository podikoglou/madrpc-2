//! MaDRPC Orchestrator
//!
//! This crate provides the orchestrator (load balancer) that forwards requests
//! to compute nodes using round-robin selection with circuit breaker and health checking.

pub mod load_balancer;
pub mod node;
pub mod health_checker;
pub mod orchestrator;

pub use load_balancer::LoadBalancer;
pub use node::{Node, DisableReason, HealthCheckStatus};
pub use health_checker::{HealthChecker, HealthCheckConfig};
pub use orchestrator::{Orchestrator, RetryConfig};
