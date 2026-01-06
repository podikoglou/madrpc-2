use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::protocol::{Request, Response};
use crate::protocol::error::{MadrpcError, Result};
use crate::transport::codec::JsonCodec;

/// Maximum message size (100 MB)
const MAX_MESSAGE_SIZE: usize = 100 * 1024 * 1024;

/// Async TCP server for MaDRPC (for Orchestrator).
///
/// This server uses tokio for async I/O and is suitable for the orchestrator
/// which handles many concurrent connections efficiently.
pub struct TcpServer {
    listener: TcpListener,
}

impl TcpServer {
    /// Creates a new TCP server bound to the specified address.
    ///
    /// # Arguments
    /// * `bind_addr` - The address to bind to (e.g., "0.0.0.0:8080")
    pub async fn new(bind_addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(bind_addr).await
            .map_err(|e| MadrpcError::Connection(format!("Failed to bind to {}: {}", bind_addr, e)))?;

        Ok(Self { listener })
    }

    /// Gets the actual bound address.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.listener.local_addr()
            .map_err(|e| MadrpcError::Connection(format!("Failed to get local addr: {}", e)))
    }

    /// Runs the server with the given request handler.
    ///
    /// This accepts connections in a loop and spawns an async task for each connection.
    /// Each connection processes multiple requests (keep-alive) until closed.
    ///
    /// # Arguments
    /// * `handler` - Function to handle each request
    pub async fn run_with_handler<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Response>> + Send + 'static,
    {
        let handler = Arc::new(handler);

        loop {
            // Accept next connection
            let (stream, peer_addr) = self.listener.accept().await
                .map_err(|e| MadrpcError::Connection(format!("Failed to accept connection: {}", e)))?;

            eprintln!("Connection established from {}", peer_addr);

            let handler = handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, handler).await {
                    eprintln!("Connection error: {}", e);
                }
            });
        }
    }
}

/// Handle a single TCP connection (async)
///
/// Processes multiple requests until the connection is closed.
async fn handle_connection<F, Fut>(
    mut stream: TcpStream,
    handler: Arc<F>,
) -> Result<()>
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Response>> + Send + 'static,
{
    loop {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Connection closed by peer
                eprintln!("Connection closed by peer");
                return Ok(());
            }
            Err(e) => {
                return Err(MadrpcError::Connection(format!("Failed to read length: {}", e)));
            }
        }

        let len = u32::from_be_bytes(len_buf) as usize;

        // Validate length
        if len > MAX_MESSAGE_SIZE {
            return Err(MadrpcError::InvalidResponse(format!(
                "Message too large: {} bytes (max {} bytes)",
                len, MAX_MESSAGE_SIZE
            )));
        }

        // Read request data
        let mut buf = vec![0u8; len];
        match stream.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(e) => {
                return Err(MadrpcError::Connection(format!("Failed to read data: {}", e)));
            }
        }

        // Decode request
        let request = match JsonCodec::decode_request(&buf) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Failed to decode request: {}", e);
                let error_response = Response::error(0, e.to_string());
                let _ = send_response(&mut stream, &error_response).await;
                continue;
            }
        };

        // Handle request
        let request_id = request.id;
        let response = match handler(request).await {
            Ok(resp) => resp,
            Err(e) => {
                eprintln!("Handler error: {}", e);
                Response::error(request_id, e.to_string())
            }
        };

        // Send response
        if let Err(e) = send_response(&mut stream, &response).await {
            eprintln!("Failed to send response: {}", e);
            return Err(e);
        }
    }
}

/// Send a response with length prefix
async fn send_response(stream: &mut TcpStream, response: &Response) -> Result<()> {
    let encoded = JsonCodec::encode_response(response)?;

    let len = encoded.len() as u32;
    stream.write_all(&len.to_be_bytes()).await
        .map_err(|e| MadrpcError::Connection(format!("Failed to send response length: {}", e)))?;
    stream.write_all(&encoded).await
        .map_err(|e| MadrpcError::Connection(format!("Failed to send response data: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tcp_server_creation() {
        let server = TcpServer::new("127.0.0.1:0").await;
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_tcp_server_local_addr() {
        let server = TcpServer::new("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr();
        assert!(addr.is_ok());
    }
}
