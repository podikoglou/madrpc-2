use madrpc_common::protocol::Request;
use madrpc_common::protocol::error::{Result, MadrpcError};
use madrpc_common::transport::QuicTransport;
use crate::pool::{ConnectionPool, PoolConfig};
use serde_json::Value;
use std::sync::Arc;

/// MaDRPC client for making RPC calls
pub struct MadrpcClient {
    orchestrator_addr: String,
    pool: Arc<ConnectionPool>,
    transport: QuicTransport,
}

impl MadrpcClient {
    /// Create a new client connected to an orchestrator
    pub async fn new(orchestrator_addr: impl Into<String>) -> Result<Self> {
        Self::with_config(orchestrator_addr, PoolConfig::default()).await
    }

    /// Create a new client with custom configuration
    pub async fn with_config(
        orchestrator_addr: impl Into<String>,
        config: PoolConfig,
    ) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let pool = Arc::new(ConnectionPool::new(config)?);
        let transport = QuicTransport::new_client()?;

        Ok(Self {
            orchestrator_addr,
            pool,
            transport,
        })
    }

    /// Call an RPC method
    pub async fn call(
        &self,
        method: impl Into<String>,
        args: Value,
    ) -> Result<Value> {
        let request = Request::new(method, args);

        // Get connection from pool
        let conn = self.pool.acquire(&self.orchestrator_addr).await?;

        // Send request via transport
        let response = self.transport.send_request(&conn.connection, &request).await?;

        // Return connection to pool
        self.pool.release(conn).await;

        // Handle response
        if response.success {
            response.result.ok_or_else(|| {
                MadrpcError::InvalidResponse("Missing result in success response".to_string())
            })
        } else {
            Err(MadrpcError::JavaScriptExecution(
                response.error.unwrap_or_else(|| "Unknown error".to_string())
            ))
        }
    }
}

impl Clone for MadrpcClient {
    fn clone(&self) -> Self {
        Self {
            orchestrator_addr: self.orchestrator_addr.clone(),
            pool: Arc::clone(&self.pool),
            transport: QuicTransport::new_client().unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Tests require a running server
    // These are basic unit tests

    #[tokio::test]
    async fn test_client_creation() {
        // Install default crypto provider for rustls
        let _ = rustls::crypto::ring::default_provider().install_default();

        let client = MadrpcClient::new("localhost:8080").await;
        // Will create successfully even if server doesn't exist
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_client_with_custom_config() {
        // Install default crypto provider for rustls
        let _ = rustls::crypto::ring::default_provider().install_default();

        let config = PoolConfig {
            max_connections: 5,
        };
        let client = MadrpcClient::with_config("localhost:8080", config).await;
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_client_is_clonable() {
        // Install default crypto provider for rustls
        let _ = rustls::crypto::ring::default_provider().install_default();

        let client = MadrpcClient::new("localhost:8080").await.unwrap();
        let client2 = client.clone();
        assert_eq!(client.orchestrator_addr, client2.orchestrator_addr);
    }
}
