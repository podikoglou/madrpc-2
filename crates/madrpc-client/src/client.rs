use madrpc_common::protocol::Request;
use madrpc_common::protocol::error::{Result, MadrpcError};
use madrpc_common::transport::TcpTransportAsync;
use serde_json::Value;
use std::sync::Arc;
use crate::pool::{ConnectionPool, PoolConfig};

/// MaDRPC client for making RPC calls
///
/// Uses a connection pool to efficiently manage TCP connections.
/// Connections are reused across requests to improve performance.
pub struct MadrpcClient {
    orchestrator_addr: String,
    pool: Arc<ConnectionPool>,
}

impl MadrpcClient {
    /// Create a new client connected to an orchestrator
    pub async fn new(orchestrator_addr: impl Into<String>) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let pool = Arc::new(ConnectionPool::new(PoolConfig::default())?);

        Ok(Self {
            orchestrator_addr,
            pool,
        })
    }

    /// Create a new client with custom pool configuration
    pub async fn with_config(orchestrator_addr: impl Into<String>, config: PoolConfig) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let pool = Arc::new(ConnectionPool::new(config)?);

        Ok(Self {
            orchestrator_addr,
            pool,
        })
    }

    /// Call an RPC method
    ///
    /// Acquires a connection from the pool, sends the request, and returns the connection to the pool.
    pub async fn call(
        &self,
        method: impl Into<String>,
        args: Value,
    ) -> Result<Value> {
        let request = Request::new(method, args);

        // Acquire connection from pool
        let conn = self.pool.acquire(&self.orchestrator_addr).await?;

        // Lock the stream for this request
        let mut stream = conn.stream.lock().await;

        // Create transport for sending request
        let transport = TcpTransportAsync::new()?;

        // Send request and get response
        let response = transport.send_request(&mut stream, &request).await?;

        // Release the stream lock
        drop(stream);

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
        let client = MadrpcClient::new("localhost:8080").await;
        // Will create successfully even if server doesn't exist
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_client_is_clonable() {
        let client = MadrpcClient::new("localhost:8080").await.unwrap();
        let client2 = client.clone();
        assert_eq!(client.orchestrator_addr, client2.orchestrator_addr);
    }
}
