//! HTTP Router for Orchestrator
//!
//! This module provides the HTTP router using the ajj JSON-RPC router.
//! It handles built-in methods (_metrics, _info) locally and forwards
//! all other methods to nodes via the fallback handler.

use ajj::{Router, ResponsePayload};
use madrpc_common::protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
use std::sync::Arc;
use serde_json::Value;

use crate::orchestrator::Orchestrator;

/// Orchestrator HTTP router with built-in methods and fallback forwarding.
///
/// This router uses the ajj JSON-RPC router to:
/// 1. Handle built-in methods (_metrics, _info) locally
/// 2. Forward all other methods to nodes via the fallback handler
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
    /// ajj router with built-in routes and fallback
    router: Router<Arc<Orchestrator>>,
}

impl OrchestratorRouter {
    /// Creates a new orchestrator router with built-in methods and fallback.
    ///
    /// # Arguments
    /// * `orchestrator` - Arc-wrapped orchestrator instance
    ///
    /// # Returns
    /// A new router instance configured with:
    /// - `_metrics` handler (built-in)
    /// - `_info` handler (built-in)
    /// - Fallback handler (forwards to nodes)
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        let router = Router::<Arc<Orchestrator>>::new()
            .route("_metrics", Self::metrics_handler)
            .route("_info", Self::info_handler)
            .fallback(|req: JsonRpcRequest, state: Arc<Orchestrator>| async move {
                Self::forward_to_node(state, req).await
            });

        Self { orchestrator, router }
    }

    /// Forwards a JSON-RPC request to a node via load balancing.
    ///
    /// This method:
    /// 1. Uses the orchestrator's load balancer to select a node
    /// 2. Forwards the request via HTTP
    /// 3. Returns the result or an error
    ///
    /// # Arguments
    /// * `orchestrator` - Orchestrator instance
    /// * `req` - JSON-RPC request to forward
    ///
    /// # Returns
    /// - `Ok(Value)` - Result from the node
    /// - `Err(ajj::Error)` - Server error if forwarding fails
    async fn forward_to_node(
        orchestrator: Arc<Orchestrator>,
        req: JsonRpcRequest,
    ) -> Result<Value, ajj::Error> {
        orchestrator
            .forward_request_jsonrpc(req)
            .await
            .map_err(|e| ajj::Error::server_error(&e.to_string()))
    }

    /// Handles _metrics requests locally.
    ///
    /// Returns orchestrator metrics including:
    /// - Request counts per node
    /// - Method call statistics
    /// - Health check status
    ///
    /// # Arguments
    /// * `orchestrator` - Orchestrator instance
    /// * `_params` - Request parameters (ignored)
    ///
    /// # Returns
    /// - `Ok(Value)` - Metrics data
    /// - `Err(ajj::Error)` - Server error if metrics collection fails
    async fn metrics_handler(
        orchestrator: Arc<Orchestrator>,
        _params: Value,
    ) -> Result<Value, ajj::Error> {
        orchestrator
            .get_metrics()
            .await
            .map_err(|e| ajj::Error::server_error(&e.to_string()))
    }

    /// Handles _info requests locally.
    ///
    /// Returns orchestrator information including:
    /// - Node count and status
    /// - Load balancer state
    /// - Circuit breaker states
    ///
    /// # Arguments
    /// * `orchestrator` - Orchestrator instance
    /// * `_params` - Request parameters (ignored)
    ///
    /// # Returns
    /// - `Ok(Value)` - Info data
    /// - `Err(ajj::Error)` - Server error if info collection fails
    async fn info_handler(
        orchestrator: Arc<Orchestrator>,
        _params: Value,
    ) -> Result<Value, ajj::Error> {
        orchestrator
            .get_info()
            .await
            .map_err(|e| ajj::Error::server_error(&e.to_string()))
    }

    /// Handles an incoming JSON-RPC request.
    ///
    /// This method routes the request to the appropriate handler:
    /// - Built-in methods (_metrics, _info) are handled locally
    /// - Other methods are forwarded to nodes via the fallback
    ///
    /// # Arguments
    /// * `req` - JSON-RPC request to handle
    ///
    /// # Returns
    /// A JSON-RPC response with the result or error
    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        self.router.handle_request(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health_checker::HealthCheckConfig;

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
        assert_eq!(router.router.routes().count(), 2); // _metrics and _info
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

        let result = OrchestratorRouter::metrics_handler(orchestrator, json!({})).await;
        assert!(result.is_ok());
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

        let result = OrchestratorRouter::info_handler(orchestrator, json!({})).await;
        assert!(result.is_ok());
    }
}
