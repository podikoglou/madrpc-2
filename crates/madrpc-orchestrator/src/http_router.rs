//! HTTP Router for Orchestrator
//!
//! This module provides the HTTP router using the ajj JSON-RPC router.
//! It handles built-in methods (_metrics, _info) locally and forwards
//! all other methods to nodes via the fallback handler.

use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use std::sync::Arc;

use crate::orchestrator::Orchestrator;

/// Orchestrator HTTP router with built-in methods and fallback forwarding.
///
/// This router:
/// 1. Handles built-in methods (_metrics, _info) locally
/// 2. Forwards all other methods to nodes via the fallback handler
///
/// # Design
///
/// The router implements the "stupid forwarder" pattern:
/// - Built-in methods are handled directly by the orchestrator
/// - Unknown methods are transparently forwarded to nodes
/// - Round-robin load balancing happens during forwarding
pub struct OrchestratorRouter {
    /// Orchestrator reference for accessing metrics and forwarding
    orchestrator: Arc<Orchestrator>,
}

impl OrchestratorRouter {
    /// Creates a new orchestrator router with built-in methods and fallback.
    ///
    /// # Arguments
    /// * `orchestrator` - Arc-wrapped orchestrator instance
    ///
    /// # Returns
    /// A new router instance
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        Self { orchestrator }
    }

    /// Handles an incoming JSON-RPC request.
    ///
    /// This method routes the request to the appropriate handler:
    /// - Built-in methods (_metrics, _info) are handled locally
    /// - Other methods are forwarded to nodes via HTTP
    ///
    /// # Arguments
    /// * `req` - JSON-RPC request to handle
    ///
    /// # Returns
    /// A JSON-RPC response with the result or error
    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        // Check for built-in methods
        match req.method.as_str() {
            "_metrics" => self.handle_metrics(req).await,
            "_info" => self.handle_info(req).await,
            _ => self.forward_to_node(req).await,
        }
    }

    /// Handles _metrics requests locally.
    ///
    /// Returns orchestrator metrics including:
    /// - Request counts per node
    /// - Method call statistics
    /// - Health check status
    ///
    /// # Arguments
    /// * `req` - JSON-RPC request
    ///
    /// # Returns
    /// A JSON-RPC response with metrics or error
    async fn handle_metrics(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match self.orchestrator.get_metrics().await {
            Ok(metrics) => JsonRpcResponse::success(req.id, metrics),
            Err(e) => {
                let error = JsonRpcError::server_error(&e.to_string());
                JsonRpcResponse::error(req.id, error)
            }
        }
    }

    /// Handles _info requests locally.
    ///
    /// Returns orchestrator information including:
    /// - Node count and status
    /// - Load balancer state
    /// - Circuit breaker states
    ///
    /// # Arguments
    /// * `req` - JSON-RPC request
    ///
    /// # Returns
    /// A JSON-RPC response with info or error
    async fn handle_info(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match self.orchestrator.get_info().await {
            Ok(info) => JsonRpcResponse::success(req.id, info),
            Err(e) => {
                let error = JsonRpcError::server_error(&e.to_string());
                JsonRpcResponse::error(req.id, error)
            }
        }
    }

    /// Forwards a JSON-RPC request to a node via load balancing.
    ///
    /// This method:
    /// 1. Uses the orchestrator's load balancer to select a node
    /// 2. Forwards the request via HTTP
    /// 3. Returns the result or an error
    ///
    /// # Arguments
    /// * `req` - JSON-RPC request to forward
    ///
    /// # Returns
    /// A JSON-RPC response with the result or error
    async fn forward_to_node(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let id = req.id.clone();
        match self.orchestrator.forward_request_jsonrpc(req).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => {
                let error = JsonRpcError::server_error(&e.to_string());
                JsonRpcResponse::error(id, error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health_checker::HealthCheckConfig;
    use serde_json::json;

    // Note: Full router tests require running nodes
    // These are basic unit tests

    #[tokio::test]
    async fn test_router_creation() {
        // Create orchestrator with no nodes (for testing router creation)
        let orchestrator = Arc::new(
            Orchestrator::with_retry_config(
                vec![],
                HealthCheckConfig::default(),
                crate::orchestrator::RetryConfig::default(),
            )
            .await
            .unwrap(),
        );

        let router = OrchestratorRouter::new(orchestrator);
        // Router created successfully
        let _ = router;
    }

    #[tokio::test]
    async fn test_metrics_handler_empty() {
        let orchestrator = Arc::new(
            Orchestrator::with_retry_config(
                vec![],
                HealthCheckConfig::default(),
                crate::orchestrator::RetryConfig::default(),
            )
            .await
            .unwrap(),
        );

        let router = OrchestratorRouter::new(orchestrator);
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "_metrics".to_string(),
            params: json!({}),
            id: json!(1),
        };

        let response = router.handle_request(req).await;
        assert_eq!(response.id, json!(1));
        // Should have result or error depending on implementation
    }

    #[tokio::test]
    async fn test_info_handler_empty() {
        let orchestrator = Arc::new(
            Orchestrator::with_retry_config(
                vec![],
                HealthCheckConfig::default(),
                crate::orchestrator::RetryConfig::default(),
            )
            .await
            .unwrap(),
        );

        let router = OrchestratorRouter::new(orchestrator);
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "_info".to_string(),
            params: json!({}),
            id: json!(1),
        };

        let response = router.handle_request(req).await;
        assert_eq!(response.id, json!(1));
        assert!(response.result.is_some() || response.error.is_some());
    }
}
