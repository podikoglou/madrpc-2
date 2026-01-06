use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::protocol::{Request, Response};
use crate::protocol::error::{Result, MadrpcError};
use crate::transport::codec::JsonCodec;

/// Default timeout for TCP operations
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// TCP transport wrapper for MaDRPC (synchronous)
pub struct TcpTransport;

impl TcpTransport {
    /// Create a new TCP transport instance
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Connect to a remote endpoint
    pub fn connect(&self, addr: &str) -> Result<TcpStream> {
        // Parse the address
        let socket_addrs = addr
            .to_socket_addrs()
            .map_err(|e| MadrpcError::Connection(format!("Invalid address '{}': {}", addr, e)))?;

        // Try each resolved address until one succeeds
        let mut last_err = None;
        for socket_addr in socket_addrs {
            match TcpStream::connect_timeout(&socket_addr, DEFAULT_TIMEOUT) {
                Ok(stream) => {
                    // Set read and write timeouts
                    stream
                        .set_read_timeout(Some(DEFAULT_TIMEOUT))
                        .map_err(|e| MadrpcError::Connection(format!("Failed to set read timeout: {}", e)))?;
                    stream
                        .set_write_timeout(Some(DEFAULT_TIMEOUT))
                        .map_err(|e| MadrpcError::Connection(format!("Failed to set write timeout: {}", e)))?;

                    return Ok(stream);
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        Err(MadrpcError::Connection(format!(
            "Failed to connect to {}: {}",
            addr,
            last_err.map(|e| e.to_string()).unwrap_or_else(|| "Unknown error".to_string())
        )))
    }

    /// Send a request and wait for response
    pub fn send_request(&self, stream: &mut TcpStream, request: &Request) -> Result<Response> {
        // Encode the request
        let encoded = JsonCodec::encode_request(request)?;

        // Send the request
        Self::send_message(stream, &encoded)?;

        // Receive the response
        let response_data = Self::receive_message(stream)?;

        // Decode the response
        let response = JsonCodec::decode_response(&response_data)?;

        Ok(response)
    }

    /// Send a message with length prefix
    ///
    /// Wire format: [4-byte length as u32 big-endian] + [data]
    pub fn send_message(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
        let len = data.len() as u32;

        // Write length prefix
        stream
            .write_all(&len.to_be_bytes())
            .map_err(|e| Self::map_io_error(e, "writing length prefix"))?;

        // Write data
        stream
            .write_all(data)
            .map_err(|e| Self::map_io_error(e, "writing data"))?;

        // Flush to ensure data is sent
        stream
            .flush()
            .map_err(|e| Self::map_io_error(e, "flushing stream"))?;

        Ok(())
    }

    /// Receive a message with length prefix
    ///
    /// Wire format: [4-byte length as u32 big-endian] + [data]
    pub fn receive_message(stream: &mut TcpStream) -> Result<Vec<u8>> {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .map_err(|e| Self::map_io_error(e, "reading length prefix"))?;

        let len = u32::from_be_bytes(len_buf) as usize;

        // Validate length to prevent allocation of excessively large buffers
        const MAX_MESSAGE_SIZE: usize = 100 * 1024 * 1024; // 100 MB
        if len > MAX_MESSAGE_SIZE {
            return Err(MadrpcError::InvalidResponse(format!(
                "Message too large: {} bytes (max {} bytes)",
                len, MAX_MESSAGE_SIZE
            )));
        }

        // Read data
        let mut buf = vec![0u8; len];
        stream
            .read_exact(&mut buf)
            .map_err(|e| Self::map_io_error(e, "reading data"))?;

        Ok(buf)
    }

    /// Map IO errors to appropriate MadrpcError variants
    fn map_io_error(err: std::io::Error, context: &str) -> MadrpcError {
        match err.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                MadrpcError::Timeout(DEFAULT_TIMEOUT.as_millis() as u64)
            }
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected => {
                MadrpcError::Connection(format!("{}: Connection lost", context))
            }
            _ => MadrpcError::Io(err),
        }
    }
}

impl Default for TcpTransport {
    fn default() -> Self {
        Self::new().expect("TcpTransport::new should never fail")
    }
}

/// Async TCP transport wrapper for MaDRPC
///
/// This is the async version of TcpTransport, used by the Orchestrator
/// which needs to maintain async/await for its operations.
pub struct TcpTransportAsync;

// Import the async traits needed for tokio::net::TcpStream
use tokio::io::{AsyncReadExt, AsyncWriteExt};

impl TcpTransportAsync {
    /// Create a new async TCP transport instance
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Connect to a remote endpoint (async)
    pub async fn connect(&self, addr: &str) -> Result<tokio::net::TcpStream> {
        // Parse the address
        let socket_addrs = addr
            .to_socket_addrs()
            .map_err(|e| MadrpcError::Connection(format!("Invalid address '{}': {}", addr, e)))?;

        // Try each resolved address until one succeeds
        let mut last_err = None;
        for socket_addr in socket_addrs {
            match tokio::net::TcpStream::connect(&socket_addr).await {
                Ok(stream) => {
                    return Ok(stream);
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        Err(MadrpcError::Connection(format!(
            "Failed to connect to {}: {}",
            addr,
            last_err.map(|e| e.to_string()).unwrap_or_else(|| "Unknown error".to_string())
        )))
    }

    /// Send a request and wait for response (async)
    pub async fn send_request(&self, stream: &mut tokio::net::TcpStream, request: &Request) -> Result<Response> {
        // Encode the request
        let encoded = JsonCodec::encode_request(request)?;

        // Send the request
        Self::send_message(stream, &encoded).await?;

        // Receive the response
        let response_data = Self::receive_message(stream).await?;

        // Decode the response
        let response = JsonCodec::decode_response(&response_data)?;

        Ok(response)
    }

    /// Send a message with length prefix (async)
    ///
    /// Wire format: [4-byte length as u32 big-endian] + [data]
    pub async fn send_message(stream: &mut tokio::net::TcpStream, data: &[u8]) -> Result<()> {
        let len = data.len() as u32;

        // Write length prefix
        stream
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| Self::map_io_error(e, "writing length prefix"))?;

        // Write data
        stream
            .write_all(data)
            .await
            .map_err(|e| Self::map_io_error(e, "writing data"))?;

        // Flush to ensure data is sent
        stream
            .flush()
            .await
            .map_err(|e| Self::map_io_error(e, "flushing stream"))?;

        Ok(())
    }

    /// Receive a message with length prefix (async)
    ///
    /// Wire format: [4-byte length as u32 big-endian] + [data]
    pub async fn receive_message(stream: &mut tokio::net::TcpStream) -> Result<Vec<u8>> {
        // Read length prefix
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| Self::map_io_error(e, "reading length prefix"))?;

        let len = u32::from_be_bytes(len_buf) as usize;

        // Validate length to prevent allocation of excessively large buffers
        const MAX_MESSAGE_SIZE: usize = 100 * 1024 * 1024; // 100 MB
        if len > MAX_MESSAGE_SIZE {
            return Err(MadrpcError::InvalidResponse(format!(
                "Message too large: {} bytes (max {} bytes)",
                len, MAX_MESSAGE_SIZE
            )));
        }

        // Read data
        let mut buf = vec![0u8; len];
        stream
            .read_exact(&mut buf)
            .await
            .map_err(|e| Self::map_io_error(e, "reading data"))?;

        Ok(buf)
    }

    /// Map IO errors to appropriate MadrpcError variants
    fn map_io_error(err: std::io::Error, context: &str) -> MadrpcError {
        match err.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                MadrpcError::Timeout(DEFAULT_TIMEOUT.as_millis() as u64)
            }
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected => {
                MadrpcError::Connection(format!("{}: Connection lost", context))
            }
            _ => MadrpcError::Io(err),
        }
    }
}

impl Default for TcpTransportAsync {
    fn default() -> Self {
        Self::new().expect("TcpTransportAsync::new should never fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_transport_creation() {
        let transport = TcpTransport::new();
        assert!(transport.is_ok());
    }

    #[test]
    fn test_tcp_transport_default() {
        let transport = TcpTransport::default();
        // Just verify it can be created
        let _ = transport;
    }

    #[test]
    fn test_tcp_transport_async_creation() {
        let transport = TcpTransportAsync::new();
        assert!(transport.is_ok());
    }

    #[test]
    fn test_tcp_transport_async_default() {
        let transport = TcpTransportAsync::default();
        // Just verify it can be created
        let _ = transport;
    }
}
