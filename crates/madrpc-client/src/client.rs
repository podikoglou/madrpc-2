use madrpc_common::protocol::Request;
use madrpc_common::protocol::error::{Result, MadrpcError};
use madrpc_common::transport::TcpTransportAsync;
use serde_json::Value;

/// MaDRPC client for making RPC calls
///
/// Creates a fresh TCP connection for each request to enable true parallelism.
/// This avoids serialization through shared Arc<Mutex<TcpStream>>.
pub struct MadrpcClient {
    orchestrator_addr: String,
    transport: TcpTransportAsync,
}

impl MadrpcClient {
    /// Create a new client connected to an orchestrator
    pub async fn new(orchestrator_addr: impl Into<String>) -> Result<Self> {
        let orchestrator_addr = orchestrator_addr.into();
        let transport = TcpTransportAsync::new()?;

        Ok(Self {
            orchestrator_addr,
            transport,
        })
    }

    /// Call an RPC method
    ///
    /// Creates a fresh TCP connection for each request, enabling true parallelism.
    pub async fn call(
        &self,
        method: impl Into<String>,
        args: Value,
    ) -> Result<Value> {
        let request = Request::new(method, args);

        // Create a fresh connection for this request
        let mut stream = self.transport.connect(&self.orchestrator_addr).await?;

        // Send request and get response
        let response = self.transport.send_request(&mut stream, &request).await?;

        // Connection is closed here when stream is dropped

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
            transport: TcpTransportAsync::new().unwrap(),
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
