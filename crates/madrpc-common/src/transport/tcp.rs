use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::protocol::{Request, Response};
use crate::protocol::error::{Result, MadrpcError};
use crate::transport::codec::JsonCodec;

/// Default timeout for TCP operations (5 seconds)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// TCP transport wrapper for MaDRPC (synchronous).
///
/// This is the synchronous TCP transport used by nodes. It provides blocking
/// I/O operations with built-in timeouts and error handling.
///
/// # Wire Protocol
///
/// Messages are sent with a 4-byte length prefix (big-endian u32) followed
/// by the JSON-encoded data:
///
/// ```text
/// [4-byte length] [JSON data]
/// ```
///
/// # Example
///
/// ```no_run
/// use madrpc_common::transport::TcpTransport;
/// use madrpc_common::protocol::Request;
/// use serde_json::json;
///
/// let transport = TcpTransport::new().unwrap();
/// let mut stream = transport.connect("127.0.0.1:8080").unwrap();
///
/// let request = Request::new("compute", json!({"n": 100}));
/// let response = transport.send_request(&mut stream, &request).unwrap();
/// ```
pub struct TcpTransport;

impl TcpTransport {
    /// Creates a new TCP transport instance.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Connects to a remote endpoint.
    ///
    /// This method resolves the address (which may resolve to multiple addresses)
    /// and attempts to connect to each until one succeeds.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address to connect to (e.g., "127.0.0.1:8080")
    ///
    /// # Returns
    ///
    /// A connected TCP stream with read/write timeouts configured
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The address cannot be parsed
    /// - Connection fails to all resolved addresses
    /// - Timeouts cannot be set on the stream
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

    /// Sends a request and waits for response.
    ///
    /// This is a convenience method that combines `send_message` and `receive_message`
    /// with JSON encoding/decoding.
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream to use
    /// * `request` - The request to send
    ///
    /// # Returns
    ///
    /// The response from the server
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

    /// Sends a message with length prefix.
    ///
    /// Wire format: `[4-byte length as u32 big-endian] + [data]`
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream to use
    /// * `data` - The data to send
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the stream fails
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

    /// Receives a message with length prefix.
    ///
    /// Wire format: `[4-byte length as u32 big-endian] + [data]`
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream to read from
    ///
    /// # Returns
    ///
    /// The received message data
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Reading the length prefix fails
    /// - Message exceeds maximum size (100 MB)
    /// - Reading the data fails
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
    ///
    /// Converts standard IO errors into domain-specific errors:
    /// - Timeouts/would block -> `Timeout`
    /// - Connection errors -> `Connection`
    /// - Other IO errors -> `Io`
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

/// Async TCP transport wrapper for MaDRPC.
///
/// This is the async version of `TcpTransport`, used by the Orchestrator
/// which needs to maintain async/await for its operations. It provides
/// the same functionality as `TcpTransport` but with async/await support.
///
/// # Wire Protocol
///
/// Messages are sent with a 4-byte length prefix (big-endian u32) followed
/// by the JSON-encoded data:
///
/// ```text
/// [4-byte length] [JSON data]
/// ```
///
/// # Example
///
/// ```no_run
/// use madrpc_common::transport::TcpTransportAsync;
/// use madrpc_common::protocol::Request;
/// use serde_json::json;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let transport = TcpTransportAsync::new()?;
/// let mut stream = transport.connect("127.0.0.1:8080").await?;
///
/// let request = Request::new("compute", json!({"n": 100}));
/// let response = transport.send_request(&mut stream, &request).await?;
/// # Ok(())
/// # }
/// ```
pub struct TcpTransportAsync;

// Import the async traits needed for tokio::net::TcpStream
use tokio::io::{AsyncReadExt, AsyncWriteExt};

impl TcpTransportAsync {
    /// Creates a new async TCP transport instance.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Connects to a remote endpoint (async).
    ///
    /// This method resolves the address and attempts to connect asynchronously.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address to connect to
    ///
    /// # Returns
    ///
    /// A connected TCP stream
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

    /// Sends a request and waits for response (async).
    ///
    /// This is a convenience method that combines `send_message` and `receive_message`
    /// with JSON encoding/decoding.
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream to use
    /// * `request` - The request to send
    ///
    /// # Returns
    ///
    /// The response from the server
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

    /// Sends a message with length prefix (async).
    ///
    /// Wire format: `[4-byte length as u32 big-endian] + [data]`
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream to use
    /// * `data` - The data to send
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

    /// Receives a message with length prefix (async).
    ///
    /// Wire format: `[4-byte length as u32 big-endian] + [data]`
    ///
    /// # Arguments
    ///
    /// * `stream` - The TCP stream to read from
    ///
    /// # Returns
    ///
    /// The received message data
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Reading the length prefix fails
    /// - Message exceeds maximum size (100 MB)
    /// - Reading the data fails
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
    ///
    /// Converts standard IO errors into domain-specific errors:
    /// - Timeouts/would block -> `Timeout`
    /// - Connection errors -> `Connection`
    /// - Other IO errors -> `Io`
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
